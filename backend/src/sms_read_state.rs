//! 插件独立已读状态；按消息指纹持久化，不把模组已读位当作客户端状态。
use crate::{
    PanelError, Result,
    sms::{Direction, Message},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub trait ReadStore: Send + Sync {
    fn load(&self) -> Result<Option<Vec<u8>>>;
    fn save(&self, raw: &[u8]) -> Result<()>;
}
#[cfg(unix)]
impl ReadStore for crate::private_store::PrivateStore {
    fn load(&self) -> Result<Option<Vec<u8>>> {
        self.load("sms-read.json")
    }
    fn save(&self, raw: &[u8]) -> Result<()> {
        self.save("sms-read.json", raw)
    }
}
fn invalid() -> PanelError {
    PanelError::new("SMS_READ_STATE_FAILED", 503)
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn scope(revision: &str) -> String {
    format!("{:x}", Sha256::digest(revision.as_bytes()))
}
fn key(message: &Message) -> Result<String> {
    message.validate()?;
    let raw = serde_json::to_vec(&json!([
        message.modem,
        message.storage,
        message.contact,
        message.timestamp,
        message.content
    ]))
    .map_err(|_| invalid())?;
    Ok(format!("{:x}", Sha256::digest(raw)))
}
type Entries = BTreeMap<String, BTreeSet<String>>;
pub struct Ledger {
    entries: Entries,
    store: Option<Arc<dyn ReadStore>>,
}
impl Ledger {
    pub fn memory() -> Self {
        Self {
            entries: BTreeMap::new(),
            store: None,
        }
    }
    pub fn open(store: Arc<dyn ReadStore>) -> Result<Self> {
        let mut entries = BTreeMap::new();
        if let Some(raw) = store.load()? {
            if raw.len() > 16384 {
                return Err(invalid());
            }
            let v: Value = serde_json::from_slice(&raw).map_err(|_| invalid())?;
            if v["version"] != 1 {
                return Err(invalid());
            }
            for (s, keys) in v["read"].as_object().ok_or_else(invalid)? {
                if !hash(s) || entries.len() >= 16 {
                    return Err(invalid());
                }
                let mut values = BTreeSet::new();
                for k in keys.as_array().ok_or_else(invalid)? {
                    let k = k.as_str().filter(|k| hash(k)).ok_or_else(invalid)?;
                    if !values.insert(k.into()) {
                        return Err(invalid());
                    }
                }
                entries.insert(s.clone(), values);
            }
            if entries.values().map(BTreeSet::len).sum::<usize>() > 128 {
                return Err(invalid());
            }
        }
        Ok(Self {
            entries,
            store: Some(store),
        })
    }
    pub fn persistent(&self) -> bool {
        self.store.is_some()
    }
    fn commit(&mut self, next: Entries) -> Result<()> {
        if next == self.entries {
            return Ok(());
        }
        if next.len() > 16 || next.values().map(BTreeSet::len).sum::<usize>() > 128 {
            return Err(PanelError::new("SMS_READ_STATE_FULL", 409));
        }
        if let Some(store) = &self.store {
            let raw =
                serde_json::to_vec(&json!({"version":1,"read":next})).map_err(|_| invalid())?;
            store.save(&raw)?;
        }
        self.entries = next;
        Ok(())
    }
    pub fn unread(&self, revision: &str, message: &Message) -> Result<bool> {
        if message.direction != Direction::Incoming {
            return Ok(false);
        }
        let k = key(message)?;
        Ok(!self
            .entries
            .get(&scope(revision))
            .is_some_and(|keys| keys.contains(&k)))
    }
    pub fn mark(&mut self, revision: &str, message: &Message, unread: bool) -> Result<()> {
        if message.direction != Direction::Incoming {
            return Err(PanelError::new("SMS_READ_STATE_INVALID", 422));
        }
        let s = scope(revision);
        let k = key(message)?;
        let mut next = self.entries.clone();
        if unread {
            if let Some(keys) = next.get_mut(&s) {
                keys.remove(&k);
                if keys.is_empty() {
                    next.remove(&s);
                }
            }
        } else {
            next.entry(s).or_default().insert(k);
        }
        self.commit(next)
    }
    pub fn reconcile(&mut self, revision: &str, messages: &[Message]) -> Result<()> {
        let mut next = self.entries.clone();
        let s = scope(revision);
        if let Some(keys) = next.get_mut(&s) {
            let active: Result<BTreeSet<_>> = messages
                .iter()
                .filter(|m| m.direction == Direction::Incoming)
                .map(key)
                .collect();
            let active = active?;
            keys.retain(|k| active.contains(k));
            if keys.is_empty() {
                next.remove(&s);
            }
        }
        self.commit(next)
    }
}
