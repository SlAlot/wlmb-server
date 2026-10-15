//! 短信公共模型、列表校验及多段消息归并。
use crate::{PanelError, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, time::Duration};

pub const MAX_MESSAGES: usize = 128;
pub const MAX_SMS_BYTES: usize = 4096;
pub const MAX_SNAPSHOT_BYTES: usize = 262_144;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Direction {
    Incoming,
    Outgoing,
    Unknown,
}
#[derive(Clone)]
pub struct Message {
    pub modem: String,
    pub storage: String,
    pub index: u32,
    pub direction: Direction,
    pub contact: String,
    pub timestamp: String,
    pub content: String,
}
impl Message {
    pub fn validate(&self) -> Result<()> {
        if self.modem.is_empty()
            || self.modem.len() > 128
            || self.storage.is_empty()
            || self.storage.len() > 16
            || !self.storage.bytes().all(|b| b.is_ascii_alphanumeric())
            || self.index > 65535
            || self.contact.is_empty()
            || self.contact.len() > 96
            || self.timestamp.len() > 96
            || self.content.is_empty()
            || self.content.len() > MAX_SMS_BYTES
        {
            return Err(PanelError::new("SMS_INVALID_RECORD", 422));
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> Result<String> {
        self.validate()?;
        let mut digest = Sha256::new();
        for part in [
            self.modem.as_bytes(),
            self.storage.as_bytes(),
            &self.index.to_be_bytes(),
            &[match self.direction {
                Direction::Incoming => 0,
                Direction::Outgoing => 1,
                Direction::Unknown => 2,
            }],
            self.contact.as_bytes(),
            self.timestamp.as_bytes(),
            self.content.as_bytes(),
        ] {
            digest.update((part.len() as u64).to_be_bytes());
            digest.update(part);
        }
        Ok(format!("{:x}", digest.finalize()))
    }
}

pub fn parse_smsd(raw: &[u8], modem: &str, storage: &str) -> Result<Vec<Message>> {
    let invalid = || PanelError::new("SMS_SCHEMA_UNAVAILABLE", 422);
    if raw.len() > MAX_SNAPSHOT_BYTES {
        return Err(PanelError::new("SMS_SNAPSHOT_TOO_LARGE", 413));
    }
    let value: Value = serde_json::from_slice(raw).map_err(|_| invalid())?;
    let groups = value["smslist"].as_array().ok_or_else(invalid)?;
    if groups.len() > MAX_MESSAGES {
        return Err(invalid());
    }
    let mut messages = Vec::new();
    let mut slots = HashSet::new();
    for group in groups {
        let contact = group["contact"].as_str().ok_or_else(invalid)?;
        for row in group["list"].as_array().ok_or_else(invalid)? {
            if messages.len() >= MAX_MESSAGES {
                return Err(PanelError::new("SMS_SNAPSHOT_TOO_LARGE", 413));
            }
            let index = row["index"]
                .as_u64()
                .filter(|n| *n <= 65535)
                .ok_or_else(invalid)? as u32;
            if !slots.insert(index) {
                return Err(invalid());
            }
            let direction = match row["direction"].as_str() {
                Some("incoming" | "received") => Direction::Incoming,
                Some("outgoing" | "sent") => Direction::Outgoing,
                Some(_) => Direction::Unknown,
                None if row["role"].as_u64() == Some(0) => Direction::Incoming,
                None if row["role"].as_u64() == Some(1) => Direction::Outgoing,
                None => Direction::Unknown,
            };
            let timestamp = match &row["timestamp"] {
                Value::String(s) => s.clone(),
                Value::Number(n) if n.as_u64().is_some() => n.to_string(),
                Value::Null => String::new(),
                _ => return Err(invalid()),
            };
            let message = Message {
                modem: modem.into(),
                storage: storage.into(),
                index,
                direction,
                contact: contact.into(),
                timestamp,
                content: row["content"].as_str().ok_or_else(invalid)?.into(),
            };
            message.validate()?;
            messages.push(message);
        }
    }
    Ok(messages)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReadMode {
    VerifiedDaemonCache,
    VerifiedDaemonEvents,
    SharedAtPolling,
}
pub struct PollPlan {
    mode: ReadMode,
    enabled: bool,
    active: Option<u64>,
    generation: u64,
    sequence: u64,
    due: Duration,
    last_finish: Duration,
    failures: u32,
    dirty: bool,
}
impl PollPlan {
    pub fn new(mode: ReadMode) -> Self {
        Self {
            mode,
            enabled: false,
            active: None,
            generation: 1,
            sequence: 0,
            due: Duration::ZERO,
            last_finish: Duration::ZERO,
            failures: 0,
            dirty: false,
        }
    }
    pub fn enable(&mut self, enabled: bool, now: Duration) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.generation = self.generation.saturating_add(1);
            self.due = now.saturating_add(Duration::from_secs(30));
            self.failures = 0;
            self.dirty = false;
        }
    }
    pub fn notify(&mut self, now: Duration) {
        if self.enabled {
            self.dirty = true;
            if self.failures > 0 {
                return;
            }
            let floor = self.last_finish.saturating_add(Duration::from_secs(30));
            self.due = self.due.min(now.max(floor));
        }
    }
    pub fn begin(&mut self, now: Duration, modem_busy: bool) -> Option<(u64, u64)> {
        if !self.enabled || self.active.is_some() || now < self.due {
            return None;
        }
        if modem_busy {
            self.due = now.saturating_add(Duration::from_secs(15));
            return None;
        }
        self.sequence = self.sequence.saturating_add(1);
        self.active = Some(self.sequence);
        self.dirty = false;
        Some((self.generation, self.sequence))
    }
    pub fn finish(&mut self, ticket: (u64, u64), now: Duration, result: Result<bool>) -> bool {
        if self.active != Some(ticket.1) {
            return false;
        }
        self.active = None;
        self.last_finish = now;
        if ticket.0 != self.generation || !self.enabled {
            self.due = now.saturating_add(Duration::from_secs(30));
            return false;
        }
        let succeeded = result.is_ok();
        let interval = match result {
            Ok(changed) => {
                self.failures = 0;
                match self.mode {
                    ReadMode::VerifiedDaemonEvents => 300,
                    ReadMode::VerifiedDaemonCache => {
                        if changed {
                            30
                        } else {
                            60
                        }
                    }
                    ReadMode::SharedAtPolling => {
                        if changed {
                            60
                        } else {
                            120
                        }
                    }
                }
            }
            Err(_) => {
                self.failures = self.failures.saturating_add(1).min(4);
                30_u64.saturating_mul(1 << self.failures).min(300)
            }
        };
        self.due = now.saturating_add(Duration::from_secs(if self.dirty && succeeded {
            30
        } else {
            interval
        }));
        true
    }
    pub fn next_due(&self) -> Option<Duration> {
        (self.enabled && self.active.is_none()).then_some(self.due)
    }
}
