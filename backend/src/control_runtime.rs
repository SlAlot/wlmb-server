//! AT控制台的会话、能力与危险操作门禁；发送前重新核验模组身份。
use crate::{
    PanelError, Result,
    radio_controls::RadioControls,
    sampler::Source,
    security::{self, Security},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
pub struct ModemControls {
    source: Arc<dyn Source>,
    security: Arc<Mutex<Security>>,
    pub radio: Arc<RadioControls>,
    pub unsafe_at: AtomicBool,
    sms_gate: Mutex<()>,
    born: Instant,
}
impl ModemControls {
    pub fn new(
        source: Arc<dyn Source>,
        security: Arc<Mutex<Security>>,
        born: Instant,
    ) -> Arc<Self> {
        Arc::new(Self {
            radio: Arc::new(RadioControls::new(
                source.clone(),
                security.clone(),
                Arc::new(AtomicBool::new(true)),
                born,
            )),
            source,
            security,
            unsafe_at: AtomicBool::new(false),
            sms_gate: Mutex::new(()),
            born,
        })
    }
    pub fn available(row: &Value) -> bool {
        row["shared"] == true && row["channelState"] != "sms-input-unknown"
            || row["transport"] == "dedicated"
                && row["identityVerified"] == true
                && row["channelReady"] == true
    }
    fn row(&self, key: &str) -> Result<Value> {
        self.source
            .modems()?
            .into_iter()
            .find(|r| r["id"] == key && Self::available(r))
            .ok_or_else(|| PanelError::new("MODEM_NOT_READY", 409))
    }
    fn binding(row: &Value, command: &str) -> Result<String> {
        let raw = serde_json::to_vec(&json!([
            "console",
            row["id"],
            row["port"],
            row["model"],
            row["instance"],
            command
        ]))
        .map_err(|_| PanelError::new("AT_INVALID", 400))?;
        Ok(format!("console:{:x}", Sha256::digest(raw)))
    }
    pub fn prepare(&self, key: &str, command: &str, owner: &str) -> Result<Value> {
        security::validate_at(command)?;
        let read = security::read_only(command);
        if !read && !self.unsafe_at.load(Ordering::Acquire) {
            return Err(PanelError::new("AT_WRITE_DISABLED", 403));
        }
        let row = self.row(key)?;
        if row["controlReadOnly"] == true && !read {
            return Err(PanelError::new("AT_WRITE_DISABLED", 403));
        }
        let mut value = self
            .security
            .lock()
            .map_err(|_| PanelError::new("CONTROL_BUSY", 409))?
            .prepare(
                owner,
                key,
                &Self::binding(&row, command)?,
                self.born.elapsed(),
            )?;
        value["readOnly"] = json!(read);
        Ok(value)
    }
    pub fn execute(&self, key: &str, command: &str, nonce: &str, owner: &str) -> Result<Value> {
        security::validate_at(command)?;
        let row = self.row(key)?;
        if row["controlReadOnly"] == true && !security::read_only(command) {
            return Err(PanelError::new("AT_WRITE_DISABLED", 403));
        }
        self.security
            .lock()
            .map_err(|_| PanelError::new("CONTROL_BUSY", 409))?
            .consume(
                nonce,
                owner,
                key,
                &Self::binding(&row, command)?,
                self.born.elapsed(),
            )?;
        if !security::read_only(command) && !self.unsafe_at.load(Ordering::Acquire) {
            return Err(PanelError::new("AT_WRITE_DISABLED", 403));
        }
        let response = self.source.query_bound(&row, command)?;
        Ok(json!({"response":response,"readOnly":security::read_only(command)}))
    }
    fn ims(&self, key: &str) -> Result<(Value, crate::sms_settings::ImsConfig)> {
        if !self.unsafe_at.load(Ordering::Acquire) {
            return Err(PanelError::new("AT_WRITE_DISABLED", 403));
        }
        let row = self.row(key)?;
        if !crate::sms_settings::supported(row["model"].as_str().unwrap_or("")) {
            return Err(PanelError::new("SMS_SETTINGS_UNSUPPORTED", 422));
        }
        let raw = self.source.query_bound(&row, "AT^IMSSWITCH?")?;
        let state = crate::sms_settings::confirmed(&raw)
            .ok_or_else(|| PanelError::new("SMS_SETTINGS_UNAVAILABLE", 422))?;
        Ok((row, state))
    }
    fn ims_binding(
        row: &Value,
        state: &crate::sms_settings::ImsConfig,
        enabled: bool,
    ) -> Result<String> {
        Self::binding(
            row,
            &format!("sms-ims:{:?}:{}", state.0, state.command(enabled)),
        )
    }
    pub fn sms_prepare(&self, key: &str, enabled: bool, owner: &str) -> Result<Value> {
        let _guard = self
            .sms_gate
            .try_lock()
            .map_err(|_| PanelError::new("CONTROL_BUSY", 409))?;
        let (row, state) = self.ims(key)?;
        let mut value = self
            .security
            .lock()
            .map_err(|_| PanelError::new("CONTROL_BUSY", 409))?
            .prepare(
                owner,
                key,
                &Self::ims_binding(&row, &state, enabled)?,
                self.born.elapsed(),
            )?;
        value["current"] = state.public();
        value["command"] = json!(state.command(enabled));
        value["warning"] = json!(
            "保存 IMS 短信配置可能影响 IMS 业务，不保证立即生效或收到短信。仅修改此开关并保留其他参数；不切飞行模式、不改 APN、不重启或自动重试。供网设备请谨慎操作。"
        );
        Ok(value)
    }
    pub fn sms_execute(&self, key: &str, enabled: bool, nonce: &str, owner: &str) -> Result<Value> {
        let _guard = self
            .sms_gate
            .try_lock()
            .map_err(|_| PanelError::new("CONTROL_BUSY", 409))?;
        let (row, state) = self.ims(key)?;
        self.security
            .lock()
            .map_err(|_| PanelError::new("CONTROL_BUSY", 409))?
            .consume(
                nonce,
                owner,
                key,
                &Self::ims_binding(&row, &state, enabled)?,
                self.born.elapsed(),
            )?;
        if !self.unsafe_at.load(Ordering::Acquire) {
            return Err(PanelError::new("AT_WRITE_DISABLED", 403));
        }
        if state.0[0] == u8::from(enabled) {
            return Ok(json!({"saved":true,"unchanged":true,"settings":state.public()}));
        }
        let reply = self.source.query_bound(&row, &state.command(enabled))?;
        if reply.lines().rfind(|s| !s.trim().is_empty()).map(str::trim) != Some("OK")
            || reply.lines().any(|s| s.contains("ERROR"))
        {
            return Err(PanelError::new("SMS_SETTINGS_UNCONFIRMED", 409));
        }
        let raw = self.source.query_bound(&row, "AT^IMSSWITCH?")?;
        let actual = crate::sms_settings::confirmed(&raw)
            .filter(|v| v.0 == [u8::from(enabled), state.0[1], state.0[2]])
            .ok_or_else(|| PanelError::new("SMS_SETTINGS_UNCONFIRMED", 409))?;
        Ok(
            json!({"saved":true,"settings":actual.public(),"activation":"unverified","deliveryVerified":false}),
        )
    }
}
