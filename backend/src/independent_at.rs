//! 自有模组通道；打开前核验身份与占用，遇到不确定回复隔离接口。
use crate::{PanelError, Result, catalog, sampler::Source};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};

pub trait Endpoint: Send {
    fn poll_notices(&mut self, _timeout: Duration) -> Result<()> {
        Ok(())
    }
    fn take_notices(&mut self) -> u64 {
        0
    }
    fn take_sms_hints(&mut self) -> crate::sms_notice::Hints {
        crate::sms_notice::Hints::default()
    }
    fn synchronize(&mut self) -> Result<()> {
        Ok(())
    }
    fn query(&mut self, command: &str, timeout: Duration) -> Result<String>;
    fn send_pdu(&mut self, pdu: &str, bytes: usize) -> Result<String>;
    fn quarantined(&self) -> bool;
}
impl<W: crate::at_channel::Wire> Endpoint for crate::at_channel::Channel<W> {
    fn poll_notices(&mut self, timeout: Duration) -> Result<()> {
        self.poll_notices(timeout)
    }
    fn take_notices(&mut self) -> u64 {
        self.take_notices()
    }
    fn take_sms_hints(&mut self) -> crate::sms_notice::Hints {
        self.take_sms_hints()
    }
    fn synchronize(&mut self) -> Result<()> {
        self.initial_quiet()
    }
    fn query(&mut self, command: &str, timeout: Duration) -> Result<String> {
        self.query(command, timeout)
    }
    fn send_pdu(&mut self, pdu: &str, bytes: usize) -> Result<String> {
        self.send_pdu(pdu, bytes)
    }
    fn quarantined(&self) -> bool {
        self.poisoned()
    }
}
pub trait Hardware: Send + Sync {
    fn inventory(&self) -> Result<Vec<Value>>;
    fn owners(&self, port: &str) -> Result<Vec<String>>;
    fn open(&self, port: &str) -> Result<Box<dyn Endpoint>>;
}
struct Entry {
    row: Value,
    endpoint: Option<Arc<Mutex<Box<dyn Endpoint>>>>,
}
#[derive(Clone)]
struct PduSnapshot {
    scope: String,
    storage: crate::sms_session::Storage,
    parts: Vec<crate::sms_pdu::Record>,
    full_at: std::time::Instant,
    incremental: bool,
}
pub struct IndependentSource {
    hardware: Arc<dyn Hardware>,
    entries: Mutex<BTreeMap<String, Entry>>,
    text_cache: Mutex<crate::sms_text::Cache>,
    pdu_cache: Mutex<BTreeMap<String, PduSnapshot>>,
    notices: crate::sms_notice::Notices,
}
fn failure(code: &'static str) -> PanelError {
    PanelError::new(code, 409)
}
pub fn identify_reply(raw: &str) -> Option<Value> {
    if raw.len() > crate::MAX_AT_RESPONSE
        || raw.lines().map(str::trim).rfind(|s| !s.is_empty()) != Some("OK")
    {
        return None;
    }
    let mut found = None;
    for line in raw.lines().map(str::trim) {
        let model = line
            .strip_prefix("+CGMM:")
            .or_else(|| line.strip_prefix("Model:"))
            .unwrap_or(line)
            .trim()
            .trim_matches('"');
        let profile = catalog::identify(model);
        if model.is_empty()
            || model.len() > 128
            || !model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            continue;
        }
        if profile["family"] == "generic" {
            continue;
        }
        if found
            .as_ref()
            .is_some_and(|p: &Value| p["model"] != profile["model"])
        {
            return None;
        }
        found = Some(profile);
    }
    found
}
impl IndependentSource {
    pub fn new(hardware: Arc<dyn Hardware>) -> Arc<Self> {
        Arc::new(Self {
            hardware,
            entries: Mutex::new(BTreeMap::new()),
            text_cache: Mutex::new(crate::sms_text::Cache::default()),
            pdu_cache: Mutex::new(BTreeMap::new()),
            notices: crate::sms_notice::Notices::default(),
        })
    }
    fn channel(&self, id: &str, port: &str, model: &str) -> Result<Arc<Mutex<Box<dyn Endpoint>>>> {
        let entries = self.entries.try_lock().map_err(|_| failure("AT_BUSY"))?;
        let entry = entries.get(id).ok_or_else(|| failure("MODEM_NOT_FOUND"))?;
        if entry.row["port"] != port || entry.row["model"] != model {
            return Err(failure("MODEM_CHANGED"));
        }
        if !self.hardware.owners(port)?.is_empty() {
            return Err(failure("AT_PORT_CONFLICT"));
        }
        entry
            .endpoint
            .clone()
            .ok_or_else(|| failure("AT_CHANNEL_UNAVAILABLE"))
    }
    pub fn send_sms_pdu(&self, id: &str, pdu: &str, bytes: usize) -> Result<String> {
        let (port, model, instance) = self.sms_identity(id)?;
        self.send_sms_pdu_bound(id, &port, &model, &instance, pdu, bytes)
    }
    fn send_sms_pdu_bound(
        &self,
        id: &str,
        port: &str,
        model: &str,
        instance: &Value,
        pdu: &str,
        bytes: usize,
    ) -> Result<String> {
        let entries = self.entries.try_lock().map_err(|_| failure("AT_BUSY"))?;
        let entry = entries.get(id).ok_or_else(|| failure("MODEM_CHANGED"))?;
        if entry.row["port"] != port
            || entry.row["model"] != model
            || &entry.row["instance"] != instance
        {
            return Err(failure("MODEM_CHANGED"));
        }
        self.verify_sms_identity(id, port, instance)?;
        if !self.hardware.owners(port)?.is_empty() {
            return Err(failure("AT_PORT_CONFLICT"));
        }
        let mut channel = entry
            .endpoint
            .as_ref()
            .ok_or_else(|| failure("AT_CHANNEL_UNAVAILABLE"))?
            .try_lock()
            .map_err(|_| failure("AT_BUSY"))?;
        let mode = channel.query("AT+CMGF?", Duration::from_secs(3))?;
        if !crate::sms_session::pdu_mode(&mode) {
            return Err(PanelError::new("SMS_PDU_MODE_REQUIRED", 422));
        }
        self.verify_sms_identity(id, port, instance)?;
        if !self.hardware.owners(port)?.is_empty() {
            return Err(failure("AT_PORT_CONFLICT"));
        }
        channel.send_pdu(pdu, bytes)
    }
}
impl Source for IndependentSource {
    fn query_bound(&self, row: &Value, command: &str) -> Result<String> {
        crate::security::validate_at(command)?;
        let id = row["id"].as_str().ok_or_else(|| failure("MODEM_CHANGED"))?;
        let port = row["port"]
            .as_str()
            .ok_or_else(|| failure("MODEM_CHANGED"))?;
        let model = row["model"]
            .as_str()
            .ok_or_else(|| failure("MODEM_CHANGED"))?;
        let entries = self.entries.try_lock().map_err(|_| failure("AT_BUSY"))?;
        let entry = entries.get(id).ok_or_else(|| failure("MODEM_NOT_FOUND"))?;
        if entry.row["instance"] != row["instance"]
            || entry.row["port"] != port
            || entry.row["model"] != model
        {
            return Err(failure("MODEM_CHANGED"));
        }
        if !self
            .hardware
            .inventory()?
            .iter()
            .any(|r| r["id"] == id && r["port"] == port && r["instance"] == row["instance"])
        {
            return Err(failure("MODEM_CHANGED"));
        }
        if !self.hardware.owners(port)?.is_empty() {
            return Err(failure("AT_PORT_CONFLICT"));
        }
        let mut endpoint = entry
            .endpoint
            .as_ref()
            .ok_or_else(|| failure("AT_CHANNEL_UNAVAILABLE"))?
            .try_lock()
            .map_err(|_| failure("AT_BUSY"))?;
        let result = endpoint.query(
            command,
            Duration::from_secs(if command.starts_with("AT+QNWLOCK=") {
                10
            } else {
                3
            }),
        );
        if endpoint.take_notices() > 0 {
            self.notices.signal(&entry.row);
        }
        if let Ok(raw) = &result {
            self.notices.observe(&entry.row, command, raw);
        }
        result
    }
    fn modems(&self) -> Result<Vec<Value>> {
        let rows = self.hardware.inventory()?;
        if rows.len() > 16 {
            return Err(failure("MODEM_INVENTORY_TOO_LARGE"));
        }
        let mut unique = BTreeSet::new();
        let mut ports = BTreeSet::new();
        for row in &rows {
            let id = row["id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .ok_or_else(|| failure("INVALID_MODEM_CONFIG"))?;
            let port = row["port"]
                .as_str()
                .ok_or_else(|| failure("INVALID_MODEM_CONFIG"))?;
            if !crate::modem_inventory::valid_port(port)
                || !unique.insert(id.to_owned())
                || !ports.insert(port.to_owned())
            {
                return Err(failure("INVALID_MODEM_CONFIG"));
            }
        }
        let mut entries = self.entries.try_lock().map_err(|_| failure("AT_BUSY"))?;
        entries.retain(|id, entry| {
            rows.iter().any(|row| {
                row["id"] == id.as_str()
                    && row["port"] == entry.row["port"]
                    && row["instance"] == entry.row["instance"]
                    && row["model"] == entry.row["configuredModel"]
            })
        });
        // 拔出或重新识别后及时释放旧分段，避免历史身份挤占缓存额度。
        self.pdu_cache
            .lock()
            .map_err(|_| failure("SMS_BUSY"))?
            .retain(|id, _| entries.contains_key(id));
        self.notices.retain(|id| entries.contains_key(id));
        let mut probed = false;
        for mut row in rows {
            let id = row["id"]
                .as_str()
                .ok_or_else(|| failure("INVALID_MODEM_CONFIG"))?
                .to_owned();
            let port = row["port"]
                .as_str()
                .ok_or_else(|| failure("INVALID_MODEM_CONFIG"))?
                .to_owned();
            if let Some(entry) = entries.get(&id) {
                if entry.row["channelState"] != "port-conflict"
                    || !self.hardware.owners(&port)?.is_empty()
                {
                    continue;
                }
                entries.remove(&id);
            }
            if probed {
                continue;
            }
            let configured = row["model"].as_str().unwrap_or("unknown").to_owned();
            row["configuredModel"] = json!(configured);
            row["transport"] = json!("dedicated");
            row["channelReady"] = json!(false);
            if !self.hardware.owners(&port)?.is_empty() {
                row["channelState"] = json!("port-conflict");
                entries.insert(
                    id,
                    Entry {
                        row,
                        endpoint: None,
                    },
                );
                continue;
            }
            probed = true;
            let opened = (|| {
                let mut endpoint = self.hardware.open(&port)?;
                if !self.hardware.owners(&port)?.is_empty() {
                    return Err(failure("AT_PORT_CONFLICT"));
                }
                endpoint.synchronize()?;
                if !self.hardware.owners(&port)?.is_empty() {
                    return Err(failure("AT_PORT_CONFLICT"));
                }
                let raw = endpoint.query("AT+CGMM", Duration::from_secs(3))?;
                let profile =
                    identify_reply(&raw).ok_or_else(|| failure("MODEM_IDENTITY_UNVERIFIED"))?;
                if configured != "unknown"
                    && catalog::identify(&configured)["model"] != profile["model"]
                {
                    return Err(failure("MODEM_CHANGED"));
                }
                Ok((endpoint, profile))
            })();
            let endpoint = match opened {
                Ok((endpoint, profile)) => {
                    row["model"] = profile["model"].clone();
                    row["profile"] = profile;
                    row["identityVerified"] = json!(true);
                    row["channelReady"] = json!(true);
                    row["channelState"] = json!("ready");
                    Some(Arc::new(Mutex::new(endpoint)))
                }
                Err(e) => {
                    row["channelState"] = json!(e.code);
                    None
                }
            };
            entries.insert(id, Entry { row, endpoint });
        }
        Ok(entries
            .values()
            .map(|entry| {
                let mut row = entry.row.clone();
                if let Some(channel) = &entry.endpoint {
                    if let Ok(channel) = channel.try_lock() {
                        if channel.quarantined() {
                            row["channelReady"] = json!(false);
                            row["channelState"] = json!("quarantined");
                        }
                    }
                }
                row
            })
            .collect())
    }
    fn query(&self, id: &str, command: &str, port: &str, model: &str) -> Result<String> {
        crate::security::validate_at(command)?;
        let instance = self
            .entries
            .lock()
            .map_err(|_| failure("AT_BUSY"))?
            .get(id)
            .map(|e| e.row["instance"].clone());
        let channel = self.channel(id, port, model)?;
        let raw = channel.try_lock().map_err(|_| failure("AT_BUSY"))?.query(
            command,
            Duration::from_secs(if command.starts_with("AT+QNWLOCK=") {
                10
            } else {
                3
            }),
        )?;
        if let Ok(entries) = self.entries.lock() {
            if let Some(entry) = entries.get(id).filter(|e| {
                e.row["port"] == port
                    && e.row["model"] == model
                    && Some(&e.row["instance"]) == instance.as_ref()
            }) {
                self.notices.observe(&entry.row, command, &raw);
            }
        }
        Ok(raw)
    }
}

impl crate::sms_runtime::SmsDevice for IndependentSource {
    fn stop_notices(&self) {
        if let Ok(mut cache) = self.pdu_cache.lock() {
            cache.clear();
        }
    }
    fn poll_notice(&self, id: &str) -> Result<()> {
        let entries = self.entries.try_lock().map_err(|_| failure("AT_BUSY"))?;
        let entry = entries.get(id).ok_or_else(|| failure("MODEM_NOT_FOUND"))?;
        let port = entry.row["port"]
            .as_str()
            .ok_or_else(|| failure("MODEM_CHANGED"))?;
        self.verify_sms_identity(id, port, &entry.row["instance"])?;
        if !self.hardware.owners(port)?.is_empty() {
            return Err(failure("AT_PORT_CONFLICT"));
        }
        let mut endpoint = entry
            .endpoint
            .as_ref()
            .ok_or_else(|| failure("AT_CHANNEL_UNAVAILABLE"))?
            .try_lock()
            .map_err(|_| failure("AT_BUSY"))?;
        endpoint.poll_notices(Duration::from_millis(50))?;
        self.verify_sms_identity(id, port, &entry.row["instance"])?;
        if endpoint.take_notices() > 0 {
            self.notices.signal(&entry.row);
        }
        Ok(())
    }
    fn notice_supported(&self) -> bool {
        true
    }
    fn notice_generation(&self, id: &str) -> u64 {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| {
                entries.get(id).map(|entry| {
                    if let Some(endpoint) = &entry.endpoint {
                        if let Ok(mut endpoint) = endpoint.try_lock() {
                            if endpoint.take_notices() > 0 {
                                self.notices.signal(&entry.row);
                            }
                        }
                    }
                    self.notices.generation(&entry.row)
                })
            })
            .unwrap_or(0)
    }
    fn revision(&self, id: &str) -> Result<String> {
        use sha2::{Digest, Sha256};
        let (port, model, instance) = self.sms_identity(id)?;
        self.verify_sms_identity(id, &port, &instance)?;
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&json!([id, port, model, instance]))
                    .map_err(|_| failure("MODEM_CHANGED"))?
            )
        ))
    }
    fn read(&self, id: &str) -> Result<Vec<crate::sms_pdu::Record>> {
        let (port, model, instance) = self.sms_identity(id)?;
        self.verify_sms_identity(id, &port, &instance)?;
        let channel = self.channel(id, &port, &model)?;
        let mut endpoint = channel.try_lock().map_err(|_| failure("AT_BUSY"))?;
        let records = self.sms_read_locked(id, &port, &model, &instance, endpoint.as_mut())?;
        self.verify_sms_identity(id, &port, &instance)?;
        Ok(records)
    }
    fn read_event(&self, id: &str) -> Result<Vec<crate::sms_pdu::Record>> {
        let (port, model, instance) = self.sms_identity(id)?;
        self.verify_sms_identity(id, &port, &instance)?;
        let channel = self.channel(id, &port, &model)?;
        let mut endpoint = channel.try_lock().map_err(|_| failure("AT_BUSY"))?;
        let hints = endpoint.take_sms_hints();
        let result =
            self.sms_incremental_locked(id, &port, &model, &instance, endpoint.as_mut(), hints);
        let records = match result {
            Ok(Some(records)) => records,
            Ok(None) => self.sms_read_locked(id, &port, &model, &instance, endpoint.as_mut())?,
            Err(e)
                if matches!(
                    e.code,
                    "AT_UNSUPPORTED" | "SMS_PDU_INVALID" | "SMS_MULTIPART_AMBIGUOUS"
                ) =>
            {
                // 索引格式不兼容时退回快照；同一通道身份不再反复试探。
                if let Some(snapshot) = self
                    .pdu_cache
                    .lock()
                    .map_err(|_| failure("SMS_BUSY"))?
                    .get_mut(id)
                {
                    snapshot.incremental = false;
                }
                self.sms_read_locked(id, &port, &model, &instance, endpoint.as_mut())?
            }
            Err(e) => return Err(e),
        };
        self.verify_sms_identity(id, &port, &instance)?;
        Ok(records)
    }
    fn delete(&self, id: &str, fingerprint: &str) -> Result<Value> {
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(failure("SMS_DELETE_INVALID"));
        }
        let (port, model, instance) = self.sms_identity(id)?;
        self.verify_sms_identity(id, &port, &instance)?;
        let channel = self.channel(id, &port, &model)?;
        let mut endpoint = channel.try_lock().map_err(|_| failure("AT_BUSY"))?;
        let rows = self.sms_read_locked(id, &port, &model, &instance, endpoint.as_mut())?;
        self.pdu_cache
            .lock()
            .map_err(|_| failure("SMS_BUSY"))?
            .remove(id);
        let target = rows
            .iter()
            .find(|r| r.message.fingerprint().is_ok_and(|f| f == fingerprint))
            .ok_or_else(|| failure("SMS_CHANGED"))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut done = 0;
        let deletion = (|| {
            for index in &target.indexes {
                self.verify_sms_identity(id, &port, &instance)?;
                if !self.hardware.owners(&port)?.is_empty() {
                    return Err(failure("AT_PORT_CONFLICT"));
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining < Duration::from_millis(100) {
                    return Err(failure("SMS_DELETE_TIMEOUT"));
                }
                let storage = crate::sms_session::storage(
                    &endpoint.query("AT+CPMS?", remaining.min(Duration::from_secs(3)))?,
                )?;
                if storage.read != target.message.storage {
                    return Err(failure("SMS_STORAGE_CHANGED"));
                }
                self.verify_sms_identity(id, &port, &instance)?;
                if !self.hardware.owners(&port)?.is_empty() {
                    return Err(failure("AT_PORT_CONFLICT"));
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining < Duration::from_millis(100) {
                    return Err(failure("SMS_DELETE_TIMEOUT"));
                }
                let command = format!("AT+CMGD={index},0");
                let answer = endpoint.query(&command, remaining.min(Duration::from_secs(3)))?;
                if !crate::controls::write_accepted(&answer, &command) {
                    return Err(failure("SMS_DELETE_UNKNOWN"));
                }
                done += 1;
            }
            Ok(())
        })();
        if let Err(e) = deletion {
            return Err(if done > 0 {
                failure("SMS_DELETE_PARTIAL")
            } else {
                e
            });
        }
        Ok(json!({"deleted":true,"parts":done}))
    }
    fn send(&self, id: &str, number: &str, content: &str) -> Result<Value> {
        let pdu = crate::sms_pdu::encode_submit(number, content)?;
        let (port, model, instance) = self.sms_identity(id)?;
        self.verify_sms_identity(id, &port, &instance)?;
        let raw =
            self.send_sms_pdu_bound(id, &port, &model, &instance, &pdu.hex, pdu.tpdu_bytes)?;
        let reference = crate::sms_session::send_reference_with_pdu_echo(&raw, &pdu.hex)?;
        Ok(json!({"sent":true,"reference":reference}))
    }
}
impl IndependentSource {
    fn sms_read_locked(
        &self,
        id: &str,
        port: &str,
        model: &str,
        instance: &Value,
        endpoint: &mut dyn Endpoint,
    ) -> Result<Vec<crate::sms_pdu::Record>> {
        // 开始快照前消费旧提示；读取期间的新通知留给下一轮处理。
        endpoint.take_sms_hints();
        let mut query = |command: &str, seconds| {
            self.verify_sms_identity(id, port, instance)?;
            if !self.hardware.owners(port)?.is_empty() {
                return Err(failure("AT_PORT_CONFLICT"));
            }
            let answer = endpoint.query(command, Duration::from_secs(seconds))?;
            self.verify_sms_identity(id, port, instance)?;
            Ok(answer)
        };
        let mode = query("AT+CMGF?", 3)?;
        if crate::sms_session::mode(&mode) == Some(1) {
            self.pdu_cache
                .lock()
                .map_err(|_| failure("SMS_BUSY"))?
                .remove(id);
            let scope = crate::sms_text::scope(&json!({
                "id": id, "port": port, "model": model, "instance": instance
            }))?;
            let mut cache = self
                .text_cache
                .try_lock()
                .map_err(|_| failure("SMS_BUSY"))?;
            cache.bind(scope);
            return crate::sms_text::read(query, id, &mut cache);
        }
        if !crate::sms_pdu::pdu_mode(&mode) {
            return Err(PanelError::new("SMS_PDU_MODE_REQUIRED", 422));
        }
        let before = crate::sms_session::storage_query(|| query("AT+CPMS?", 3))?;
        let raw = query("AT+CMGL=4", 10)?;
        let after = crate::sms_session::storage_query(|| query("AT+CPMS?", 3))?;
        let after_mode = query("AT+CMGF?", 3)?;
        if before != after || !crate::sms_pdu::pdu_mode(&after_mode) {
            return Err(failure("SMS_STORAGE_CHANGED"));
        }
        let parts = crate::sms_pdu::parse_cmgl_parts(&raw, id, &before.read)?;
        let records = crate::sms_pdu::assemble(parts.clone())?;
        let scope = crate::sms_text::scope(
            &json!({"id":id,"port":port,"model":model,"instance":instance}),
        )?;
        let mut cache = self.pdu_cache.lock().map_err(|_| failure("SMS_BUSY"))?;
        if !cache.contains_key(id) && cache.len() >= 16 {
            return Err(failure("SMS_BUSY"));
        }
        let incremental = cache
            .get(id)
            .filter(|c| c.scope == scope)
            .is_none_or(|c| c.incremental);
        cache.insert(
            id.into(),
            PduSnapshot {
                scope,
                storage: before,
                parts,
                full_at: std::time::Instant::now(),
                incremental,
            },
        );
        Ok(records)
    }
    fn sms_incremental_locked(
        &self,
        id: &str,
        port: &str,
        model: &str,
        instance: &Value,
        endpoint: &mut dyn Endpoint,
        hints: crate::sms_notice::Hints,
    ) -> Result<Option<Vec<crate::sms_pdu::Record>>> {
        let scope = crate::sms_text::scope(
            &json!({"id":id,"port":port,"model":model,"instance":instance}),
        )?;
        let Some(mut snapshot) = self
            .pdu_cache
            .lock()
            .map_err(|_| failure("SMS_BUSY"))?
            .get(id)
            .filter(|c| c.scope == scope)
            .cloned()
        else {
            return Ok(None);
        };
        // 提示丢失、积压或旧快照均走完整核对，避免事件持续到来时饿死兜底扫描。
        if !snapshot.incremental
            || hints.overflow
            || hints.indexes.is_empty()
            || hints.indexes.len() > 8
            || snapshot.full_at.elapsed() >= Duration::from_secs(10)
        {
            return Ok(None);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut query = |command: &str| {
            self.verify_sms_identity(id, port, instance)?;
            if !self.hardware.owners(port)?.is_empty() {
                return Err(failure("AT_PORT_CONFLICT"));
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining < Duration::from_millis(100) {
                return Err(failure("SMS_BUSY"));
            }
            let raw = endpoint.query(command, remaining.min(Duration::from_secs(3)))?;
            self.verify_sms_identity(id, port, instance)?;
            if !self.hardware.owners(port)?.is_empty() {
                return Err(failure("AT_PORT_CONFLICT"));
            }
            Ok(raw)
        };
        if !crate::sms_session::pdu_mode(&query("AT+CMGF?")?) {
            return Ok(None);
        }
        let before = crate::sms_session::storage(&query("AT+CPMS?")?)?;
        if snapshot.storage != before
            || hints.indexes.iter().any(|(store, _)| store != &before.read)
        {
            return Ok(None);
        }
        for (_, index) in hints.indexes {
            let raw = query(&format!("AT+CMGR={index}"))?;
            let record = crate::sms_pdu::parse_cmgr(&raw, id, &before.read, index)?;
            snapshot.parts.retain(|r| r.message.index != index);
            snapshot.parts.push(record);
            if snapshot.parts.len() > crate::sms::MAX_MESSAGES {
                return Ok(None);
            }
        }
        let after = crate::sms_session::storage(&query("AT+CPMS?")?)?;
        if before != after || !crate::sms_session::pdu_mode(&query("AT+CMGF?")?) {
            return Err(failure("SMS_STORAGE_CHANGED"));
        }
        let records = crate::sms_pdu::assemble(snapshot.parts.clone())?;
        self.pdu_cache
            .lock()
            .map_err(|_| failure("SMS_BUSY"))?
            .insert(id.into(), snapshot);
        Ok(Some(records))
    }
    fn sms_identity(&self, id: &str) -> Result<(String, String, Value)> {
        let entries = self.entries.try_lock().map_err(|_| failure("AT_BUSY"))?;
        let entry = entries.get(id).ok_or_else(|| failure("MODEM_NOT_FOUND"))?;
        if entry.endpoint.is_none() || entry.row["identityVerified"] != true {
            return Err(failure("AT_CHANNEL_UNAVAILABLE"));
        }
        Ok((
            entry.row["port"].as_str().unwrap_or("").into(),
            entry.row["model"].as_str().unwrap_or("").into(),
            entry.row["instance"].clone(),
        ))
    }
    fn verify_sms_identity(&self, id: &str, port: &str, instance: &Value) -> Result<()> {
        if self
            .hardware
            .inventory()?
            .iter()
            .any(|r| r["id"] == id && r["port"] == port && &r["instance"] == instance)
        {
            Ok(())
        } else {
            Err(failure("MODEM_CHANGED"))
        }
    }
}

#[cfg(unix)]
pub struct SystemHardware;
#[cfg(unix)]
impl Hardware for SystemHardware {
    fn inventory(&self) -> Result<Vec<Value>> {
        use std::{
            fs,
            os::unix::fs::{FileTypeExt, MetadataExt},
            path::Path,
        };
        let mut rows = if Path::new("/etc/config/network_panel").exists() {
            crate::modem_inventory::parse_owned_config(&crate::transport::process(
                "/sbin/uci",
                &["-q", "show", "network_panel"],
                None,
                Duration::from_secs(3),
            )?)?
        } else {
            vec![]
        };
        if rows.is_empty() {
            let candidates = crate::modem_inventory::scan_sysfs(Path::new("/sys"))?;
            let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            for row in candidates {
                if crate::modem_inventory::at_candidate(&row) {
                    grouped
                        .entry(row["physicalDevice"].as_str().unwrap_or("").to_owned())
                        .or_default()
                        .push(row);
                }
            }
            for (physical, mut candidates) in grouped {
                if candidates.len() != 1 {
                    continue;
                }
                let mut row = candidates.remove(0);
                use sha2::{Digest, Sha256};
                let key = format!("{:x}", Sha256::digest(physical.as_bytes()));
                row["id"] = json!(format!("auto-{}", &key[..24]));
                row["model"] = json!("unknown");
                rows.push(row);
            }
        }
        for row in &mut rows {
            let port = row["port"]
                .as_str()
                .ok_or_else(|| failure("INVALID_MODEM_CONFIG"))?;
            let meta = fs::symlink_metadata(port).map_err(|_| failure("AT_CHANNEL_UNAVAILABLE"))?;
            if !meta.file_type().is_char_device() {
                return Err(failure("AT_INVALID_PORT"));
            }
            row["instance"] = json!(format!("{}:{}", meta.rdev(), meta.ino()));
        }
        Ok(rows)
    }
    fn owners(&self, port: &str) -> Result<Vec<String>> {
        use crate::transport::Discovery;
        crate::transport::SystemDiscovery::default().owners(port)
    }
    fn open(&self, port: &str) -> Result<Box<dyn Endpoint>> {
        Ok(Box::new(crate::at_channel::native::open(port)?))
    }
}
