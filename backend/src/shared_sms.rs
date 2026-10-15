//! 已核验共享服务的短信事务租约与原始发送，防止跨客户端交错。
use crate::{PanelError, Result, transport::Broker};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
pub fn raw_supported(schema: &Value) -> bool {
    [
        ("at_port", 3),
        ("at_cmd", 3),
        ("raw_at_content", 3),
        ("end_flag", 3),
        ("timeout", 5),
    ]
    .iter()
    .all(|(field, kind)| schema["sendat"][*field] == *kind)
}
pub fn wire_raw(payload: &str) -> Result<String> {
    if payload.is_empty() || payload.len() > 2048 || !payload.is_ascii() {
        return Err(PanelError::new("SMS_PAYLOAD_INVALID", 422));
    }
    Ok(payload.bytes().map(|b| format!("{b:02X}")).collect())
}
pub fn supported(schema: &Value) -> bool {
    [
        ("sendat", "at_port", 3),
        ("sendat", "at_cmd", 3),
        ("sendat", "raw_at_content", 3),
        ("sendat", "end_flag", 3),
        ("sendat", "owner", 3),
        ("sendat", "timeout", 5),
        ("lease_acquire", "at_port", 3),
        ("lease_acquire", "owner", 3),
        ("lease_acquire", "ttl", 5),
        ("lease_release", "at_port", 3),
        ("lease_release", "owner", 3),
        ("lease_release", "token", 3),
    ]
    .iter()
    .all(|(method, field, kind)| schema[*method][*field] == *kind)
}
pub(crate) struct Lease<'a> {
    pub broker: &'a Broker,
    pub row: &'a Value,
    pub owner: String,
    token: String,
    pub deadline: Instant,
    pub input_pending: bool,
    pub held: Option<Arc<crate::qmodem_queue::Held>>,
}
impl<'a> Lease<'a> {
    pub fn acquire(broker: &'a Broker, row: &'a Value) -> Result<Self> {
        let held = broker.queue_hold(
            row["port"]
                .as_str()
                .ok_or_else(|| PanelError::new("AT_NOT_READY", 409))?,
        )?;
        if !broker.sms_leased {
            if held.is_none() {
                return Err(PanelError::new("SMS_SHARED_READ_ONLY", 422));
            }
            return Ok(Self {
                broker,
                row,
                owner: String::new(),
                token: String::new(),
                deadline: Instant::now() + Duration::from_secs(60),
                input_pending: false,
                held,
            });
        }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| PanelError::new("RANDOM_UNAVAILABLE", 503))?;
        let owner = format!(
            "wlmb-sms-{}",
            nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let answer = broker.bus.call(
            "at-daemon",
            "lease_acquire",
            json!({"at_port":row["port"],"owner":owner,"ttl":120}),
            Duration::from_secs(3),
        )?;
        if answer["status"] != "success" {
            return Err(PanelError::new("SMS_SHARED_BUSY", 409));
        }
        let token = answer["token"]
            .as_str()
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 96
                    && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            .ok_or_else(|| PanelError::new("SHARED_TRANSACTIONS_UNAVAILABLE", 422))?
            .to_owned();
        Ok(Self {
            broker,
            row,
            owner,
            token,
            deadline: Instant::now() + Duration::from_secs(60),
            input_pending: false,
            held,
        })
    }
    pub fn query(&self, command: &str, seconds: u64) -> Result<String> {
        self.broker.sms_query(self, command, None, seconds, None)
    }
    pub fn send(&mut self, pdu: &crate::sms_pdu::Submit) -> Result<Value> {
        if !crate::sms_session::pdu_mode(&self.query("AT+CMGF?", 3)?) {
            return Err(PanelError::new("SMS_PDU_MODE_REQUIRED", 422));
        }
        let command = format!("AT+CMGS={}", pdu.tpdu_bytes);
        self.input_pending = true;
        let prompt = self.broker.sms_query(self, &command, None, 3, Some(">"))?;
        let stripped = prompt
            .trim()
            .strip_prefix(&command)
            .unwrap_or(prompt.trim())
            .trim();
        if stripped != ">" {
            return Err(PanelError::new("SMS_PROMPT_UNKNOWN", 503));
        }
        let raw = self.broker.sms_query(
            self,
            &command,
            Some(&format!("{}\u{1a}", pdu.hex)),
            10,
            None,
        )?;
        let reference = crate::sms_session::send_reference_with_pdu_echo(&raw, &pdu.hex)?;
        self.input_pending = false;
        Ok(json!({"sent":true,"reference":reference,"transport":"shared-serialized"}))
    }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if self.input_pending {
            let cancelled = self.broker.sms_query(self, "AT", Some("\u{1b}"), 3, None);
            if !cancelled
                .as_ref()
                .is_ok_and(|raw| crate::controls::write_accepted(raw, ""))
            {
                self.broker.sms_quarantine(self.row);
                if let Some(held) = self.held.take() {
                    self.broker.retain_queue(self.row, held);
                }
                return;
            }
        }
        if self.owner.is_empty() {
            return;
        }
        let _ = self.broker.bus.call(
            "at-daemon",
            "lease_release",
            json!({"at_port":self.row["port"],"owner":self.owner,"token":self.token}),
            Duration::from_secs(3),
        );
    }
}
