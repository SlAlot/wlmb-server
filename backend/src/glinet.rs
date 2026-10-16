//! GL.iNet 原厂共享查询通道；按服务发现总线，绝不打开或重置原厂串口。
use crate::{PanelError, Result, sampler::Source, sms_runtime::SmsDevice, transport::Bus};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn error(code: &'static str) -> PanelError {
    PanelError::new(code, 409)
}
pub fn object_name(name: &str) -> bool {
    name.strip_prefix("modem.")
        .and_then(|s| s.strip_suffix(".AT"))
        .is_some_and(|bus| {
            !bus.is_empty()
                && bus.len() <= 64
                && bus
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
        })
}
pub fn request_valid(value: &Value) -> bool {
    value.as_object().is_some_and(|v| v.len() == 2)
        && value["cmd"]
            .as_str()
            .is_some_and(|cmd| crate::security::validate_at(cmd).is_ok())
        && value["timeout"]
            .as_u64()
            .is_some_and(|n| (1..=10).contains(&n))
}
fn supported(row: &Value) -> bool {
    row["name"].as_str().is_some_and(object_name)
        && row["objectId"]
            .as_u64()
            .is_some_and(|id| id > 0 && id <= u32::MAX.into())
        && row["signature"]["get_result_AT"]["cmd"] == 3
        && row["signature"]["get_result_AT"]["timeout"] == 5
}
fn response(value: Value) -> Result<String> {
    let raw = value["data"]
        .as_str()
        .ok_or_else(|| error("AT_RESPONSE_MISMATCH"))?;
    if value["channel_status"] != true || raw.len() > crate::MAX_AT_RESPONSE || raw.contains('\0') {
        return Err(error("AT_FAILED"));
    }
    let lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if lines
        .iter()
        .any(|s| *s == "ERROR" || s.starts_with("+CME ERROR:") || s.starts_with("+CMS ERROR:"))
    {
        return Err(PanelError::new("AT_UNSUPPORTED", 422));
    }
    if lines.last() != Some(&"OK") || lines.iter().filter(|s| **s == "OK").count() != 1 {
        return Err(error("AT_RESPONSE_MISMATCH"));
    }
    Ok(raw.to_owned())
}
fn model_reply(raw: &str) -> Option<Value> {
    let rows: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "OK" && *s != "AT+CGMM")
        .collect();
    if rows.len() != 1 {
        return None;
    }
    let model = rows[0]
        .strip_prefix("+CGMM:")
        .unwrap_or(rows[0])
        .trim()
        .trim_matches('"');
    if model.is_empty()
        || model.len() > 128
        || !model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return None;
    }
    Some(crate::catalog::identify(model))
}
#[derive(Default)]
struct Cache {
    rows: Vec<Value>,
    objects: Vec<Value>,
    at: Option<Instant>,
    last_scan: Option<Instant>,
}
pub struct Glinet {
    bus: Arc<dyn Bus>,
    cache: Mutex<Cache>,
    gate: Mutex<()>,
}
impl Glinet {
    pub fn new(bus: Arc<dyn Bus>) -> Arc<Self> {
        Arc::new(Self {
            bus,
            cache: Mutex::new(Cache::default()),
            gate: Mutex::new(()),
        })
    }
    pub fn detected(bus: &dyn Bus) -> Result<bool> {
        match bus.objects("modem.*.AT", Duration::from_secs(3)) {
            Ok(rows) if rows.len() <= 8 => Ok(rows
                .iter()
                .any(|r| r["name"].as_str().is_some_and(object_name))),
            Err(e) if e.code == "UBUS_OBJECT_NOT_FOUND" => Ok(false),
            Err(e) => Err(e),
            _ => Err(error("MODEM_DISCOVERY_UNAVAILABLE")),
        }
    }
    fn objects(&self) -> Result<Vec<Value>> {
        let mut rows = self.bus.objects("modem.*.AT", Duration::from_secs(2))?;
        if rows.len() > 8 {
            return Err(error("MODEM_DISCOVERY_UNAVAILABLE"));
        }
        let mut names = BTreeSet::new();
        for row in &rows {
            let name = row["name"]
                .as_str()
                .ok_or_else(|| error("MODEM_DISCOVERY_UNAVAILABLE"))?;
            if !object_name(name) || !names.insert(name) {
                return Err(error("MODEM_DISCOVERY_UNAVAILABLE"));
            }
        }
        rows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Ok(rows)
    }
    fn call(&self, object: &str, command: &str, seconds: u64) -> Result<String> {
        crate::security::validate_at(command)?;
        response(self.bus.call(
            object,
            "get_result_AT",
            json!({"cmd":command,"timeout":seconds}),
            Duration::from_secs(seconds + 2),
        )?)
    }
    fn row(&self, key: &str) -> Result<Value> {
        {
            let cache = self.cache.lock().map_err(|_| error("AT_BUSY"))?;
            if cache
                .at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(60))
            {
                return cache
                    .rows
                    .iter()
                    .find(|r| r["id"] == key)
                    .cloned()
                    .ok_or_else(|| error("MODEM_NOT_FOUND"));
            }
        }
        self.modems()?
            .into_iter()
            .find(|r| r["id"] == key)
            .ok_or_else(|| error("MODEM_NOT_FOUND"))
    }
    fn unchanged(&self, row: &Value) -> Result<()> {
        if self
            .objects()?
            .iter()
            .any(|r| supported(r) && r["name"] == row["object"] && r["objectId"] == row["instance"])
        {
            Ok(())
        } else {
            Err(error("MODEM_CHANGED"))
        }
    }
    fn query_locked(
        &self,
        row: &Value,
        command: &str,
        seconds: u64,
        inbox: bool,
    ) -> Result<String> {
        let read = crate::security::read_only(command)
            || ["AT+CNMI?", "AT+CSMS?"].contains(&command)
            || inbox && command == "AT+CMGL=4";
        if !read {
            return Err(PanelError::new("AT_WRITE_DISABLED", 403));
        }
        self.unchanged(row)?;
        let raw = self.call(
            row["object"]
                .as_str()
                .ok_or_else(|| error("MODEM_CHANGED"))?,
            command,
            seconds,
        )?;
        self.unchanged(row)?;
        if !inbox && !crate::sampling::reply_matches_query(command, &raw) {
            return Err(error("AT_RESPONSE_MISMATCH"));
        }
        Ok(raw)
    }
}
impl Source for Glinet {
    fn modems(&self) -> Result<Vec<Value>> {
        let objects = self.objects()?;
        {
            let cache = self.cache.lock().map_err(|_| error("AT_BUSY"))?;
            if cache.objects == objects
                && cache
                    .at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(60))
            {
                return Ok(cache.rows.clone());
            }
            if cache
                .last_scan
                .is_some_and(|at| at.elapsed() < Duration::from_secs(10))
            {
                return Err(error("AT_COOLDOWN"));
            }
        }
        let _gate = self.gate.try_lock().map_err(|_| error("AT_BUSY"))?;
        self.cache.lock().map_err(|_| error("AT_BUSY"))?.last_scan = Some(Instant::now());
        let mut rows = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut failure = None;
        for object in objects.iter().filter(|r| supported(r)) {
            let name = object["name"]
                .as_str()
                .ok_or_else(|| error("MODEM_DISCOVERY_UNAVAILABLE"))?;
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .as_secs()
                .saturating_sub(2)
                .min(5);
            if remaining == 0 {
                failure = Some(error("AT_COOLDOWN"));
                break;
            }
            let raw = match self.call(name, "AT+CGMM", remaining) {
                Ok(raw) => raw,
                Err(e) => {
                    failure = Some(e);
                    continue;
                }
            };
            let Some(profile) = model_reply(&raw) else {
                continue;
            };
            rows.push(json!({"id":format!("glinet-{}",name.strip_prefix("modem.").and_then(|s|s.strip_suffix(".AT")).unwrap_or("")),
                "object":name,"port":format!("ubus:{name}"),"instance":object["objectId"],"model":profile["model"],"profile":profile,
                "shared":true,"transport":"glinet-ubus","sharedService":name,"channelReady":true,"identityVerified":true,
                "controlReadOnly":true,"smsReadSupported":profile["family"]!="generic","channelState":"ready","discoveredBy":"glinet-ubus"}));
        }
        // 扫描期间服务更换时不发布混合实例，也不自动改走独立串口。
        if self.objects()? != objects {
            return Err(error("MODEM_CHANGED"));
        }
        if rows.is_empty() {
            if let Some(e) = failure {
                return Err(e);
            }
        }
        let mut cache = self.cache.lock().map_err(|_| error("AT_BUSY"))?;
        cache.rows = rows.clone();
        cache.objects = objects;
        cache.at = Some(Instant::now());
        Ok(rows)
    }
    fn query(&self, id: &str, command: &str, port: &str, model: &str) -> Result<String> {
        let row = self.row(id)?;
        if row["port"] != port || row["model"] != model {
            return Err(error("MODEM_CHANGED"));
        }
        self.query_bound(&row, command)
    }
    fn query_bound(&self, row: &Value, command: &str) -> Result<String> {
        let current = self.row(row["id"].as_str().unwrap_or(""))?;
        if ["port", "model", "instance"]
            .iter()
            .any(|k| current[*k] != row[*k])
        {
            return Err(error("MODEM_CHANGED"));
        }
        let _gate = self.gate.try_lock().map_err(|_| error("AT_BUSY"))?;
        self.query_locked(row, command, 5, false)
    }
}
impl SmsDevice for Glinet {
    fn writable(&self) -> bool {
        false
    }
    fn source_info(&self, _modem: &str) -> Value {
        json!({"source":"glinet-shared-at","mode":"pdu-poll","eventConnected":false,"deleteScope":"unavailable","writeReason":"unverified-prompt-and-transaction-isolation"})
    }
    fn revision(&self, key: &str) -> Result<String> {
        use sha2::{Digest, Sha256};
        let row = self.row(key)?;
        self.unchanged(&row)?;
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&json!([row["id"], row["model"], row["instance"]]))
                    .map_err(|_| error("MODEM_CHANGED"))?
            )
        ))
    }
    fn read(&self, key: &str) -> Result<Vec<crate::sms_pdu::Record>> {
        let row = self.row(key)?;
        if row["smsReadSupported"] == false {
            return Err(PanelError::new("SMS_READ_UNSUPPORTED", 422));
        }
        let _gate = self.gate.try_lock().map_err(|_| error("AT_BUSY"))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let query = |command: &str, desired: u64| {
            let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
            if remaining <= 6 {
                return Err(error("AT_BUSY"));
            }
            let value = self.query_locked(
                &row,
                command,
                desired.min(remaining - 6),
                command == "AT+CMGL=4",
            )?;
            if Instant::now() > deadline {
                return Err(error("AT_BUSY"));
            }
            Ok(value)
        };
        let mode = || query("AT+CMGF?", 5u64);
        if !crate::sms_session::pdu_mode(&mode()?) {
            return Err(PanelError::new("SMS_PDU_MODE_REQUIRED", 422));
        }
        let before = crate::sms_session::storage(&query("AT+CPMS?", 5)?)?;
        let raw = query("AT+CMGL=4", 10)?;
        let after = crate::sms_session::storage(&query("AT+CPMS?", 5)?)?;
        if before != after || !crate::sms_session::pdu_mode(&mode()?) {
            return Err(error("SMS_STORAGE_CHANGED"));
        }
        crate::sms_pdu::parse_cmgl(&raw, key, &before.read)
    }
    fn send(&self, _id: &str, _number: &str, _content: &str) -> Result<Value> {
        Err(PanelError::new("SMS_SHARED_READ_ONLY", 422))
    }
}
