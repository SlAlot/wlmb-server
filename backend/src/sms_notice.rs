//! 短信URC索引提示与溢出标记，通知不足时触发完整快照核对。
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct Notices(Mutex<BTreeMap<String, (String, u64)>>);
#[derive(Clone, Default)]
pub struct Hints {
    pub indexes: std::collections::BTreeSet<(String, u32)>,
    pub overflow: bool,
}
impl Hints {
    pub fn observe(&mut self, raw: &str) {
        if raw.len() > crate::MAX_AT_RESPONSE {
            self.overflow = true;
            return;
        }
        for line in raw.lines() {
            if let Some(hint) = cmti(line) {
                if self.indexes.contains(&hint) {
                    continue;
                }
                if self.indexes.len() >= 32 {
                    self.overflow = true;
                } else {
                    self.indexes.insert(hint);
                }
            }
        }
    }
}
pub fn cmti(line: &str) -> Option<(String, u32)> {
    let (store, index) = line.trim().strip_prefix("+CMTI:")?.trim().split_once(',')?;
    let store = store.trim();
    let index = index.trim();
    if !["\"SM\"", "\"ME\"", "\"MT\"", "\"SR\"", "\"BM\"", "\"TA\""].contains(&store)
        || index.is_empty()
        || !index.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((
        store.trim_matches('"').into(),
        index.parse::<u32>().ok().filter(|v| *v <= 65535)?,
    ))
}
pub fn metadata_or_non_sms(command: &str) -> bool {
    !command.starts_with("AT+CM")
        || ["AT+CMGF?", "AT+CPMS?", "AT+CSCS?", "AT+CSDH?"].contains(&command)
}
#[derive(Default)]
pub struct SharedCursor {
    scope: String,
    epoch: u64,
    sequence: u64,
    dropped: u64,
}
impl SharedCursor {
    pub fn observe(&mut self, row: &Value, event: &Value) -> bool {
        if event["port"] != row["port"] || event["correlation"] != "idle" {
            return false;
        }
        let Some((_, scope)) = scope(row) else {
            return false;
        };
        let (Some(epoch), Some(sequence), Some(dropped)) = (
            event["restart_epoch"].as_u64(),
            event["sequence"].as_u64(),
            event["drop_count"].as_u64(),
        ) else {
            return false;
        };
        if epoch == 0 || sequence == 0 {
            return false;
        }
        if self.scope != scope {
            *self = Self {
                scope,
                ..Self::default()
            };
        }
        if self.epoch == epoch && sequence <= self.sequence {
            return false;
        }
        let gap = self.epoch != 0 && (self.epoch != epoch || dropped > self.dropped);
        self.epoch = epoch;
        self.sequence = sequence;
        self.dropped = dropped;
        let cmti = event["raw_line"]
            .as_str()
            .is_some_and(|s| s.len() <= 256 && !s.contains(['\r', '\n']) && has_cmti(s));
        cmti || gap
    }
}
fn scope(row: &Value) -> Option<(String, String)> {
    let id = row["id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)?;
    Some((
        id.into(),
        serde_json::to_string(&[&row["port"], &row["model"], &row["instance"]]).ok()?,
    ))
}
pub fn has_cmti(raw: &str) -> bool {
    if raw.len() > crate::MAX_AT_RESPONSE {
        return false;
    }
    raw.lines().any(|line| cmti(line).is_some())
}
impl Notices {
    pub fn retain(&self, live: impl Fn(&str) -> bool) {
        if let Ok(mut entries) = self.0.lock() {
            entries.retain(|id, _| live(id));
        }
    }
    pub fn generation_id(&self, id: &str) -> u64 {
        self.0
            .lock()
            .ok()
            .and_then(|entries| entries.get(id).map(|entry| entry.1))
            .unwrap_or(0)
    }
    pub fn observe(&self, row: &Value, command: &str, raw: &str) {
        if !metadata_or_non_sms(command) || !has_cmti(raw) {
            return;
        }
        self.signal(row);
    }
    pub fn signal(&self, row: &Value) {
        let Some((id, signature)) = scope(row) else {
            return;
        };
        if let Ok(mut entries) = self.0.lock() {
            if !entries.contains_key(&id) && entries.len() >= 16 {
                return;
            }
            let entry = entries.entry(id).or_insert((signature.clone(), 0));
            if entry.0 != signature {
                *entry = (signature, 0);
            }
            entry.1 = entry.1.saturating_add(1);
        }
    }
    pub fn generation(&self, row: &Value) -> u64 {
        let Some((id, signature)) = scope(row) else {
            return 0;
        };
        self.0
            .lock()
            .ok()
            .and_then(|entries| {
                entries
                    .get(&id)
                    .filter(|entry| entry.0 == signature)
                    .map(|entry| entry.1)
            })
            .unwrap_or(0)
    }
}
pub fn fast_scan_allowed(notice: u64, consumed: u64, last: Instant, now: Instant) -> bool {
    notice > consumed && now.saturating_duration_since(last) >= Duration::from_secs(2)
}
