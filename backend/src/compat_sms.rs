//! 旧共享通道的短信适配；读写语义与跨客户端保护必须经核验。
use crate::{PanelError, Result, sms_pdu::Record, sms_runtime::SmsDevice, transport::Broker};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
pub struct CompatSms(
    pub Arc<Broker>,
    Mutex<crate::sms_text::Cache>,
    Mutex<EventState>,
);
#[derive(Default)]
struct EventState {
    connection: Option<Box<dyn crate::transport::SmsEvents>>,
    cursor: crate::sms_notice::SharedCursor,
    retry: Option<std::time::Instant>,
}
use crate::sms_session::{pdu_mode, storage_query};
fn unavailable() -> PanelError {
    PanelError::new("SMS_STORAGE_UNAVAILABLE", 422)
}
impl CompatSms {
    pub fn new(broker: Arc<Broker>) -> Self {
        Self(
            broker,
            Mutex::new(crate::sms_text::Cache::default()),
            Mutex::new(EventState::default()),
        )
    }
    fn identity(&self, id: &str) -> Result<Value> {
        self.0
            .modems()?
            .into_iter()
            .find(|r| r["id"] == id && r["shared"] == true)
            .ok_or_else(|| PanelError::new("AT_NOT_SHARED", 409))
    }
    fn query(&self, row: &Value, cmd: &str) -> Result<String> {
        self.query_timeout(
            row,
            cmd,
            if matches!(cmd, "AT+CMGL=4" | "AT+CMGL=\"ALL\"") {
                10
            } else {
                3
            },
        )
    }
    fn query_timeout(&self, row: &Value, cmd: &str, seconds: u64) -> Result<String> {
        for attempt in 0..4 {
            let result = self.0.query_bound(row, cmd, seconds);
            if !matches!(&result,Err(e) if e.code=="AT_BUSY") || attempt == 3 {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(100 * (attempt + 1)));
        }
        Err(PanelError::new("AT_BUSY", 409))
    }
}
impl SmsDevice for CompatSms {
    fn notice_idle_delay(&self) -> std::time::Duration {
        if self.notice_supported() {
            std::time::Duration::from_millis(100)
        } else if self.0.sms_events {
            std::time::Duration::from_secs(1)
        } else {
            std::time::Duration::from_secs(60)
        }
    }
    fn poll_notice(&self, id: &str) -> Result<()> {
        if !self.0.sms_events {
            return Ok(());
        }
        let row = self.identity(id)?;
        let mut state = self
            .2
            .try_lock()
            .map_err(|_| PanelError::new("SMS_BUSY", 409))?;
        if state.connection.is_none()
            && state
                .retry
                .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(60))
        {
            state.retry = Some(std::time::Instant::now());
            state.connection = self.0.bus.sms_events().ok();
        }
        for index in 0..32 {
            let Some(connection) = state.connection.as_mut() else {
                break;
            };
            match connection.next(if index == 0 {
                std::time::Duration::from_secs(1)
            } else {
                std::time::Duration::from_millis(1)
            }) {
                Ok(Some(event)) => {
                    if state.cursor.observe(&row, &event) {
                        self.0.notices.signal(&row);
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    state.connection = None;
                    state.retry = Some(std::time::Instant::now());
                    self.0.notices.signal(&row);
                    break;
                }
            }
        }
        Ok(())
    }
    fn stop_notices(&self) {
        if let Ok(mut state) = self.2.lock() {
            *state = EventState::default();
        }
    }
    fn notice_generation(&self, id: &str) -> u64 {
        self.0.notices.generation_id(id)
    }
    fn notice_supported(&self) -> bool {
        self.2.lock().is_ok_and(|state| state.connection.is_some())
    }
    fn writable(&self) -> bool {
        self.0.sms_write_supported()
    }
    fn revision(&self, id: &str) -> Result<String> {
        let r = self.identity(id)?;
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&json!([r["id"], r["port"], r["model"], r["instance"]]))
                    .map_err(|_| unavailable())?
            )
        ))
    }
    fn read(&self, id: &str) -> Result<Vec<Record>> {
        let row = self.identity(id)?;
        let mode = self.query(&row, "AT+CMGF?")?;
        if crate::sms_session::mode(&mode) == Some(1) {
            let mut cache = self
                .1
                .try_lock()
                .map_err(|_| PanelError::new("SMS_BUSY", 409))?;
            cache.bind(crate::sms_text::scope(&row)?);
            return crate::sms_text::read(
                |cmd, seconds| self.query_timeout(&row, cmd, seconds),
                id,
                &mut cache,
            );
        }
        if !pdu_mode(&mode) {
            return Err(PanelError::new("SMS_PDU_MODE_REQUIRED", 422));
        }
        let before = storage_query(|| self.query(&row, "AT+CPMS?"))?;
        let raw = self.query(&row, "AT+CMGL=4")?;
        let after = storage_query(|| self.query(&row, "AT+CPMS?"))?;
        let mode = self.query(&row, "AT+CMGF?")?;
        if before != after || !pdu_mode(&mode) {
            return Err(PanelError::new("SMS_STORAGE_CHANGED", 409));
        }
        crate::sms_pdu::parse_cmgl(&raw, id, &before.read)
    }
    fn send(&self, id: &str, number: &str, content: &str) -> Result<Value> {
        let row = self.identity(id)?;
        let pdu = crate::sms_pdu::encode_submit(number, content)?;
        self.0.sms_transaction(&row, |lease| lease.send(&pdu))
    }
    fn delete(&self, id: &str, fingerprint: &str) -> Result<Value> {
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(PanelError::new("SMS_DELETE_INVALID", 422));
        }
        let row = self.identity(id)?;
        self.0.sms_transaction(&row, |lease| {
            if !pdu_mode(&lease.query("AT+CMGF?", 3)?) {
                return Err(PanelError::new("SMS_PDU_MODE_REQUIRED", 422));
            }
            let before = crate::sms_session::storage(&lease.query("AT+CPMS?", 3)?)?;
            let raw = lease.query("AT+CMGL=4", 10)?;
            let after = crate::sms_session::storage(&lease.query("AT+CPMS?", 3)?)?;
            if before != after {
                return Err(PanelError::new("SMS_STORAGE_CHANGED", 409));
            }
            let records = crate::sms_pdu::parse_cmgl(&raw, id, &before.read)?;
            let target = records
                .iter()
                .find(|r| r.message.fingerprint().is_ok_and(|f| f == fingerprint))
                .ok_or_else(|| PanelError::new("SMS_CHANGED", 409))?;
            let mut done = 0;
            for index in &target.indexes {
                let command = format!("AT+CMGD={index},0");
                let result = (|| {
                    let current = crate::sms_session::storage(&lease.query("AT+CPMS?", 3)?)?;
                    if current != before {
                        return Err(PanelError::new("SMS_STORAGE_CHANGED", 409));
                    }
                    if !pdu_mode(&lease.query("AT+CMGF?", 3)?) {
                        return Err(PanelError::new("SMS_PDU_MODE_REQUIRED", 422));
                    }
                    let raw = lease.query(&command, 3)?;
                    if !crate::controls::write_accepted(&raw, &command) {
                        return Err(PanelError::new("SMS_DELETE_UNKNOWN", 503));
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    return Err(if done > 0 {
                        PanelError::new("SMS_DELETE_PARTIAL", 503)
                    } else {
                        e
                    });
                }
                done += 1;
            }
            Ok(json!({"deleted":true,"parts":done,"transport":"shared-serialized"}))
        })
    }
}
