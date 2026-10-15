//! 参考QModem公开qmodem.sms第三版接口；独立Rust客户端，来源及语义见docs/QMODEM.md。
use crate::{
    PanelError, Result,
    sms::{Direction, Message},
    sms_pdu::Record,
    sms_runtime::SmsDevice,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn unavailable() -> PanelError {
    PanelError::new("SMS_SERVICE_UNAVAILABLE", 503)
}
fn changed() -> PanelError {
    PanelError::new("SMS_STORAGE_CHANGED", 409)
}
fn signature(row: &Value) -> Option<String> {
    if row["shared"] != true
        || row["discoveredBy"] != "qmodem-config"
        || !row["instance"].is_string()
    {
        return None;
    }
    let bytes = serde_json::to_vec(&json!([
        row["id"],
        row["port"],
        row["smsPort"],
        row["model"],
        row["instance"]
    ]))
    .ok()?;
    Some(format!("{:x}", Sha256::digest(bytes)))
}
pub trait Service: Send + Sync {
    fn identity(&self, modem: &str) -> Result<Value>;
    fn describe(&self) -> Result<Value>;
    fn call(&self, method: &str, params: Value) -> Result<Value>;
    fn event(&self) -> Result<Option<Value>> {
        Ok(None)
    }
    fn event_timeout(&self, _timeout: Duration) -> Result<Option<Value>> {
        self.event()
    }
    fn event_connected(&self) -> bool {
        false
    }
    fn stop_events(&self) {}
}
#[derive(Clone)]
struct Route {
    signature: String,
    database: Option<Database>,
    checked: Instant,
}
#[derive(Clone)]
struct Database {
    mode: String,
    writable: bool,
}
#[derive(Default)]
struct State {
    routes: BTreeMap<String, Route>,
    targets: BTreeMap<String, BTreeMap<String, u64>>,
    events: BTreeMap<String, (u64, u64, u64)>,
    followup: BTreeMap<String, Instant>,
}
pub struct QmodemSms {
    service: Arc<dyn Service>,
    fallback: Arc<dyn SmsDevice>,
    state: Mutex<State>,
}
impl QmodemSms {
    pub fn new(service: Arc<dyn Service>, fallback: Arc<dyn SmsDevice>) -> Self {
        Self {
            service,
            fallback,
            state: Mutex::new(State::default()),
        }
    }
    fn identity(&self, modem: &str) -> Result<(Value, String)> {
        if modem.is_empty()
            || modem.len() >= 64
            || !modem
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(unavailable());
        }
        let row = self.service.identity(modem)?;
        if row["id"] != modem
            || row["shared"] != true
            || row["discoveredBy"] != "qmodem-config"
            || !row["instance"].is_string()
        {
            return Err(unavailable());
        }
        let signature = signature(&row).ok_or_else(unavailable)?;
        Ok((row, signature))
    }
    fn route(&self, modem: &str) -> Result<Route> {
        // 来源：QModem公开的qmodem.sms接口；只有明确不存在才走旧AT兼容。
        let (_, signature) = self.identity(modem)?;
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if let Some(route) = state.routes.get(modem).filter(|r| {
            r.signature == signature
                && (r.database.is_some() || r.checked.elapsed() < Duration::from_secs(60))
        }) {
            return Ok(route.clone());
        }
        if state.routes.len() >= 16 && !state.routes.contains_key(modem) {
            return Err(unavailable());
        }
        let database = match self.service.describe() {
            Err(e) if e.code == "UBUS_OBJECT_NOT_FOUND" => None,
            Err(e) => return Err(e),
            Ok(schema) => {
                if !schema["status"].is_object()
                    || !policy(&schema, "config_get", &[("modem_id", 3)])
                    || !policy(
                        &schema,
                        "list",
                        &[("modem_id", 3), ("limit", 5), ("offset", 5)],
                    )
                    || !policy(&schema, "sync", &[("modem_id", 3)])
                {
                    return Err(unavailable());
                }
                let status = self.service.call("status", json!({}))?;
                if status["status"] != "ready" || status["api_version"] != 3 {
                    return Err(unavailable());
                }
                let config = self.service.call("config_get", json!({"modem_id":modem}))?;
                if config["status"] != "success" {
                    return Err(unavailable());
                }
                match config["mode"].as_str() {
                    Some("database_poll" | "database_urc") => Some(Database {
                        mode: config["mode"].as_str().ok_or_else(unavailable)?.into(),
                        writable: policy(
                            &schema,
                            "send_managed",
                            &[
                                ("modem_id", 3),
                                ("recipient", 3),
                                ("content", 3),
                                ("request_id", 3),
                            ],
                        ) && policy(&schema, "get", &[("modem_id", 3), ("id", 0)])
                            && policy(&schema, "delete", &[("modem_id", 3), ("id", 0)]),
                    }),
                    Some("direct") => None,
                    _ => return Err(unavailable()),
                }
            }
        };
        let route = Route {
            signature,
            database,
            checked: Instant::now(),
        };
        if route.database.is_some() {
            self.fallback.stop_notices();
        }
        state.targets.remove(modem);
        state.routes.insert(modem.into(), route.clone());
        Ok(route)
    }
    fn collect(&self, modem: &str, synchronize: bool) -> Result<Vec<Record>> {
        // 上游sync沿用用户原存储策略，可能移除SIM副本；本插件不改策略。
        let route = self.route(modem)?;
        if route.database.is_none() {
            return self.fallback.read(modem);
        }
        let database = self.check(modem, &route)?;
        if synchronize || database.mode != "database_urc" || !self.service.event_connected() {
            let result = self.service.call("sync", json!({"modem_id":modem}))?;
            if result["status"] != "success" {
                return Err(unavailable());
            }
        }
        let mut records = Vec::new();
        let mut targets = BTreeMap::new();
        let mut ids = BTreeSet::new();
        let mut bytes = 0;
        for page in 0..16 {
            let snapshot = self
                .service
                .call("list", json!({"modem_id":modem,"limit":8,"offset":page*8}))?;
            if snapshot["mode"] != database.mode {
                return Err(changed());
            }
            let rows = snapshot["messages"]
                .as_array()
                .filter(|v| v.len() <= 8)
                .ok_or_else(unavailable)?;
            for row in rows {
                let id = row["id"].as_u64().ok_or_else(unavailable)?;
                if !ids.insert(id) {
                    return Err(unavailable());
                }
                if let Some((id, record)) = record(row, modem)? {
                    bytes += record.message.content.len()
                        + record.message.contact.len()
                        + record.message.timestamp.len();
                    if bytes > crate::sms::MAX_SNAPSHOT_BYTES {
                        return Err(unavailable());
                    }
                    targets.insert(record.message.fingerprint()?, id);
                    records.push(record);
                }
            }
            if rows.len() < 8 {
                break;
            }
        }
        self.check(modem, &route)?;
        self.state
            .lock()
            .map_err(|_| unavailable())?
            .targets
            .insert(modem.into(), targets);
        Ok(records)
    }
    fn check(&self, modem: &str, route: &Route) -> Result<Database> {
        if self.identity(modem)?.1 != route.signature {
            return Err(PanelError::new("MODEM_CHANGED", 409));
        }
        let database = route.database.clone().ok_or_else(unavailable)?;
        let config = self.service.call("config_get", json!({"modem_id":modem}))?;
        if config["status"] != "success" || config["mode"] != database.mode {
            return Err(changed());
        }
        Ok(database)
    }
    pub fn source(&self, modem: &str) -> Value {
        self.state.lock().ok().and_then(|s|s.routes.get(modem).cloned()).map_or(json!({"source":"pending"}),|r| if r.database.is_some(){json!({"source":"qmodem-database","deleteScope":"qmodem-record","windowLimit":128,"incompletePolicy":"wait-for-assembly"})}else{json!({"source":"shared-at","deleteScope":"modem-storage"})})
    }
}
fn policy(schema: &Value, method: &str, fields: &[(&str, u8)]) -> bool {
    schema[method].is_object()
        && fields
            .iter()
            .all(|(name, kind)| schema[method][*name] == *kind)
}
fn flag(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(v) => Some(*v),
        Value::Number(v) if v.as_u64() == Some(0) => Some(false),
        Value::Number(v) if v.as_u64() == Some(1) => Some(true),
        _ => None,
    }
}
pub fn record(row: &Value, modem: &str) -> Result<Option<(u64, Record)>> {
    let id = row["id"]
        .as_u64()
        .filter(|v| *v > 0 && *v <= i64::MAX as u64)
        .ok_or_else(unavailable)?;
    if !flag(&row["complete"]).ok_or_else(unavailable)? {
        return Ok(None);
    }
    let (direction, contact) = match row["type"].as_str() {
        Some("received") => (Direction::Incoming, &row["sender"]),
        Some("sent") => (Direction::Outgoing, &row["recipient"]),
        _ => return Err(unavailable()),
    };
    let timestamp = row["timestamp"]
        .as_u64()
        .filter(|v| *v <= 253402300799)
        .ok_or_else(unavailable)?;
    let message = Message {
        modem: modem.into(),
        storage: format!("QDB{:x}", id >> 16),
        index: (id & 65535) as u32,
        direction,
        contact: contact.as_str().ok_or_else(unavailable)?.into(),
        timestamp: timestamp.to_string(),
        content: row["content"].as_str().ok_or_else(unavailable)?.into(),
    };
    message.validate()?;
    Ok(Some((
        id,
        Record {
            indexes: vec![message.index],
            message,
            concatenation: None,
            dcs: 0,
        },
    )))
}
impl SmsDevice for QmodemSms {
    fn writable(&self) -> bool {
        false
    }
    fn writable_for(&self, modem: &str) -> bool {
        self.route(modem).is_ok_and(|r| {
            r.database
                .map_or_else(|| self.fallback.writable_for(modem), |d| d.writable)
        })
    }
    fn writable_snapshot(&self, row: &Value) -> bool {
        // 展示复用已发现的同一身份；发送、删除仍走实时复核，不放宽写权限。
        let Some(modem) = row["id"].as_str() else {
            return false;
        };
        let signature = signature(row);
        let cached = self.state.lock().ok().and_then(|state| {
            state
                .routes
                .get(modem)
                .filter(|route| {
                    Some(route.signature.as_str()) == signature.as_deref()
                        && (route.database.is_some()
                            || route.checked.elapsed() < Duration::from_secs(60))
                })
                .cloned()
        });
        match cached {
            Some(route) => route.database.map_or_else(
                || self.fallback.writable_for(modem),
                |database| database.writable,
            ),
            None => self.writable_for(modem),
        }
    }
    fn source_info(&self, modem: &str) -> Value {
        self.source(modem)
    }
    fn revision(&self, modem: &str) -> Result<String> {
        let r = self.route(modem)?;
        Ok(format!(
            "{}:{}",
            r.signature,
            r.database.map_or_else(|| "shared-at".into(), |d| d.mode)
        ))
    }
    fn read(&self, modem: &str) -> Result<Vec<Record>> {
        self.collect(modem, true)
    }
    fn read_event(&self, modem: &str) -> Result<Vec<Record>> {
        self.collect(modem, false)
    }
    fn send(&self, modem: &str, number: &str, content: &str) -> Result<Value> {
        // 来源：QModem公开managed发送契约；超时／部分发送不回退、不重试。
        let route = self.route(modem)?;
        if route.database.is_none() {
            return self.fallback.send(modem, number, content);
        }
        if !self.check(modem, &route)?.writable {
            return Err(PanelError::new("SMS_SHARED_READ_ONLY", 422));
        }
        crate::sms_pdu::encode_submit(number, content)?;
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| unavailable())?;
        let request_id: String = nonce.iter().map(|v| format!("{v:02x}")).collect();
        let reply=self.service.call("send_managed",json!({"modem_id":modem,"recipient":number,"content":content,"request_id":request_id})).map_err(|_|PanelError::new("SMS_SEND_UNKNOWN",503))?;
        if reply["status"] != "success" {
            return Err(PanelError::new("SMS_SEND_UNKNOWN", 503));
        }
        Ok(json!({"sent":true,"source":"qmodem-database"}))
    }
    fn delete(&self, modem: &str, fingerprint: &str) -> Result<Value> {
        // 数据库完整ID与SIM索引隔离；复核指纹后只删除选中的数据库记录。
        let route = self.route(modem)?;
        if route.database.is_none() {
            return self.fallback.delete(modem, fingerprint);
        }
        if !self.check(modem, &route)?.writable {
            return Err(PanelError::new("SMS_SHARED_READ_ONLY", 422));
        }
        let id = self
            .state
            .lock()
            .map_err(|_| unavailable())?
            .targets
            .get(modem)
            .and_then(|m| m.get(fingerprint))
            .copied()
            .ok_or_else(changed)?;
        let current = self
            .service
            .call("get", json!({"modem_id":modem,"id":id}))?;
        let (current_id, current) = record(&current, modem)?.ok_or_else(changed)?;
        if current_id != id || current.message.fingerprint()? != fingerprint {
            return Err(changed());
        }
        self.check(modem, &route)?;
        let reply = self
            .service
            .call("delete", json!({"modem_id":modem,"id":id}))
            .map_err(|_| PanelError::new("SMS_DELETE_UNKNOWN", 503))?;
        if reply["status"] != "success" || reply["deleted"] != 1 {
            return Err(PanelError::new("SMS_DELETE_UNKNOWN", 503));
        }
        self.state
            .lock()
            .map_err(|_| unavailable())?
            .targets
            .get_mut(modem)
            .map(|m| m.remove(fingerprint));
        Ok(json!({"deleted":true,"deleteScope":"qmodem-record"}))
    }
    fn poll_notice(&self, modem: &str) -> Result<()> {
        {
            let mut state = self.state.lock().map_err(|_| unavailable())?;
            if state
                .followup
                .get(modem)
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(2))
            {
                state.followup.remove(modem);
                if let Some(cursor) = state.events.get_mut(modem) {
                    cursor.2 = cursor.2.saturating_add(1);
                }
            }
        }
        let route = {
            self.state
                .lock()
                .map_err(|_| unavailable())?
                .routes
                .get(modem)
                .cloned()
        };
        if route.as_ref().is_none_or(|r| r.database.is_none()) {
            return self.fallback.poll_notice(modem);
        }
        let route = route.ok_or_else(unavailable)?;
        let mut row = None;
        // 有界合并一批通知，只做一次身份发现；无事件时阻塞等待，不循环发AT。
        for index in 0..32 {
            let Some(event) = self.service.event_timeout(if index == 0 {
                Duration::from_secs(1)
            } else {
                Duration::from_millis(1)
            })?
            else {
                break;
            };
            if event["owner"] != "qmodem.sms" || event["urc_id"] != "sms-new" {
                continue;
            }
            if row.is_none() {
                let identity = self.service.identity(modem)?;
                if signature(&identity).as_deref() != Some(route.signature.as_str()) {
                    return Err(changed());
                }
                row = Some(identity);
            }
            let row = row.as_ref().ok_or_else(unavailable)?;
            let port = row["smsPort"]
                .as_str()
                .or_else(|| row["port"].as_str())
                .ok_or_else(unavailable)?;
            if event["port"] != port
                || event["owner"] != "qmodem.sms"
                || event["urc_id"] != "sms-new"
            {
                continue;
            }
            let epoch = event["restart_epoch"]
                .as_u64()
                .filter(|v| *v > 0)
                .ok_or_else(unavailable)?;
            let seq = event["sequence"]
                .as_u64()
                .filter(|v| *v > 0)
                .ok_or_else(unavailable)?;
            let mut state = self.state.lock().map_err(|_| unavailable())?;
            let cursor = state.events.entry(modem.into()).or_insert((0, 0, 0));
            if epoch > cursor.0 || epoch == cursor.0 && seq > cursor.1 {
                cursor.0 = epoch;
                cursor.1 = seq;
                cursor.2 = cursor.2.saturating_add(1);
                state
                    .followup
                    .entry(modem.into())
                    .or_insert_with(Instant::now);
            }
        }
        Ok(())
    }
    fn notice_generation(&self, modem: &str) -> u64 {
        self.state
            .lock()
            .ok()
            .and_then(|s| s.events.get(modem).map(|v| v.2))
            .unwrap_or(0)
            .saturating_add(self.fallback.notice_generation(modem))
    }
    fn notice_supported(&self) -> bool {
        self.service.event_connected() || self.fallback.notice_supported()
    }
    fn notice_idle_delay(&self) -> Duration {
        if self.service.event_connected() {
            Duration::from_millis(100)
        } else {
            self.fallback.notice_idle_delay()
        }
    }
    fn stop_notices(&self) {
        self.service.stop_events();
        self.fallback.stop_notices();
    }
}

#[cfg(unix)]
type NativeEvents = (
    Option<Box<dyn crate::transport::SmsEvents>>,
    Option<Instant>,
);
#[cfg(unix)]
pub struct NativeService {
    broker: Arc<crate::transport::Broker>,
    events: Mutex<NativeEvents>,
}
#[cfg(unix)]
impl NativeService {
    pub fn new(broker: Arc<crate::transport::Broker>) -> Self {
        Self {
            broker,
            events: Mutex::new((None, None)),
        }
    }
}
#[cfg(unix)]
impl Service for NativeService {
    fn identity(&self, modem: &str) -> Result<Value> {
        self.broker
            .modems()?
            .into_iter()
            .find(|r| r["id"] == modem)
            .ok_or_else(unavailable)
    }
    fn describe(&self) -> Result<Value> {
        self.broker
            .bus
            .describe("qmodem.sms", Duration::from_secs(3))
    }
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.broker.bus.call(
            "qmodem.sms",
            method,
            params,
            Duration::from_secs(if matches!(method, "sync" | "send_managed") {
                30
            } else {
                5
            }),
        )
    }
    fn event(&self) -> Result<Option<Value>> {
        self.event_timeout(Duration::from_secs(1))
    }
    fn event_timeout(&self, timeout: Duration) -> Result<Option<Value>> {
        let mut state = self.events.lock().map_err(|_| unavailable())?;
        if state.0.is_none()
            && state
                .1
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(60))
        {
            state.1 = Some(Instant::now());
            state.0 = self.broker.bus.qmodem_sms_events().ok();
        }
        let Some(channel) = state.0.as_mut() else {
            return Ok(None);
        };
        match channel.next(timeout) {
            Ok(v) => Ok(v),
            Err(e) => {
                state.0 = None;
                state.1 = Some(Instant::now());
                Err(e)
            }
        }
    }
    fn event_connected(&self) -> bool {
        self.events.lock().is_ok_and(|s| s.0.is_some())
    }
    fn stop_events(&self) {
        if let Ok(mut state) = self.events.lock() {
            *state = (None, None);
        }
    }
}
