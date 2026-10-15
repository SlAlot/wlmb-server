//! 短信基线、持久去重与PushPlus投递；摘要日志不保存正文和凭据。
use crate::{
    PanelError, Result,
    sms::{Direction, MAX_MESSAGES, Message},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

pub const PUSHPLUS_URL: &str = "https://www.pushplus.plus/send";
pub const MAX_PENDING: usize = 32;
pub const MAX_LEDGER: usize = 1024;
pub const MAX_JOURNAL_BYTES: usize = 131_072;
pub const MAX_PUSH_BODY: usize = 32768;
pub const MIN_PUSH_SECONDS: u64 = 13;
pub const MAX_PUSH_BATCH: usize = 8;

fn invalid() -> PanelError {
    PanelError::new("SMS_FORWARD_INVALID", 422)
}
fn journal_error() -> PanelError {
    PanelError::new("SMS_FORWARD_JOURNAL_FAILED", 503)
}
fn scope_hash(modem: &str) -> Result<String> {
    if modem.is_empty() || modem.len() > 128 {
        return Err(invalid());
    }
    Ok(format!("{:x}", Sha256::digest(modem.as_bytes())))
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub trait Journal {
    fn save(&mut self, checkpoint: &[u8]) -> Result<()>;
}
pub trait PushClient {
    fn send(&self, message: &Message) -> PushOutcome;
    fn can_batch(&self, messages: &[Message]) -> bool {
        messages.len() == 1
    }
    fn send_batch(&self, messages: &[Message]) -> PushOutcome {
        match messages {
            [message] => self.send(message),
            _ => PushOutcome::Unknown,
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Accepted(String),
    Rejected,
    Unknown,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Delivery {
    Baseline,
    Pending,
    InFlight,
    Accepted,
    Rejected,
    Unknown,
}
impl Delivery {
    fn label(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Pending => "pending",
            Self::InFlight => "inFlight",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "baseline" => Self::Baseline,
            "pending" => Self::Pending,
            "inFlight" | "unknown" => Self::Unknown,
            "accepted" => Self::Accepted,
            "rejected" => Self::Rejected,
            _ => return Err(invalid()),
        })
    }
}
pub struct Forwarder {
    ledger: BTreeMap<String, Delivery>,
    baselines: BTreeSet<String>,
    queue: VecDeque<(String, Message)>,
    enabled: bool,
    next_send: Duration,
}
impl Default for Forwarder {
    fn default() -> Self {
        Self {
            ledger: BTreeMap::new(),
            baselines: BTreeSet::new(),
            queue: VecDeque::new(),
            enabled: false,
            next_send: Duration::ZERO,
        }
    }
}
impl Forwarder {
    pub fn restore(raw: &[u8]) -> Result<Self> {
        if raw.len() > MAX_JOURNAL_BYTES {
            return Err(invalid());
        }
        let value: Value = serde_json::from_slice(raw).map_err(|_| invalid())?;
        if value["version"] != 1 {
            return Err(invalid());
        }
        let records = value["records"].as_object().ok_or_else(invalid)?;
        let baselines = value["baselines"].as_array().ok_or_else(invalid)?;
        if records.len() > MAX_LEDGER || baselines.len() > 16 {
            return Err(invalid());
        }
        let mut result = Self::default();
        for (key, status) in records {
            if !hash(key) {
                return Err(invalid());
            }
            result.ledger.insert(
                key.clone(),
                Delivery::parse(status.as_str().ok_or_else(invalid)?)?,
            );
        }
        for scope in baselines {
            let scope = scope.as_str().filter(|s| hash(s)).ok_or_else(invalid)?;
            if !result.baselines.insert(scope.into()) {
                return Err(invalid());
            }
        }
        Ok(result)
    }
    pub fn checkpoint(&self) -> Result<Vec<u8>> {
        let records: BTreeMap<&str, &str> = self
            .ledger
            .iter()
            .map(|(key, state)| (key.as_str(), state.label()))
            .collect();
        let raw =
            serde_json::to_vec(&json!({"version":1,"baselines":self.baselines,"records":records}))
                .map_err(|_| journal_error())?;
        if raw.len() > MAX_JOURNAL_BYTES {
            return Err(journal_error());
        }
        Ok(raw)
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.queue.clear();
            for state in self.ledger.values_mut() {
                if *state == Delivery::Pending {
                    *state = Delivery::Baseline;
                }
            }
        }
    }
    pub fn observe(&mut self, modem: &str, messages: &[Message]) -> Result<usize> {
        let scope = scope_hash(modem)?;
        if messages.len() > MAX_MESSAGES {
            return Err(invalid());
        }
        let first_scan = !self.baselines.contains(&scope);
        if first_scan && self.baselines.len() >= 16 {
            return Err(invalid());
        }
        let mut keys = BTreeSet::new();
        let mut incoming = Vec::new();
        for message in messages {
            message.validate()?;
            if message.modem != modem {
                return Err(invalid());
            }
            let key = message.fingerprint()?;
            if !keys.insert(key.clone()) {
                return Err(invalid());
            }
            if message.direction == Direction::Incoming {
                incoming.push((key, message));
            }
        }
        let additions = incoming
            .iter()
            .filter(|(key, _)| !self.ledger.contains_key(key))
            .count();
        if self.ledger.len() + additions > MAX_LEDGER {
            return Err(PanelError::new("SMS_FORWARD_HISTORY_FULL", 503));
        }
        let queued = incoming
            .iter()
            .filter(|(key, _)| {
                !self.queue.iter().any(|(queued, _)| queued == key)
                    && ((!self.ledger.contains_key(key) && !first_scan)
                        || self.ledger.get(key) == Some(&Delivery::Pending))
            })
            .count();
        if self.enabled && !first_scan && self.queue.len() + queued > MAX_PENDING {
            return Err(PanelError::new("SMS_FORWARD_QUEUE_FULL", 503));
        }
        let mut added = 0;
        for (key, message) in incoming {
            let state = self
                .ledger
                .entry(key.clone())
                .or_insert(if first_scan || !self.enabled {
                    Delivery::Baseline
                } else {
                    Delivery::Pending
                });
            if !self.enabled && *state == Delivery::Pending {
                *state = Delivery::Baseline;
            }
            if self.enabled
                && *state == Delivery::Pending
                && !self.queue.iter().any(|(queued, _)| queued == &key)
            {
                self.queue.push_back((key, message.clone()));
                added += 1;
            }
        }
        self.baselines.insert(scope);
        Ok(added)
    }
    pub fn dispatch(
        &mut self,
        now: Duration,
        journal: &mut dyn Journal,
        client: &dyn PushClient,
    ) -> Result<bool> {
        if !self.enabled || now < self.next_send {
            return Ok(false);
        }
        let Some((key, message)) = self.queue.front() else {
            return Ok(false);
        };
        let mut keys = vec![key.clone()];
        let mut messages = vec![message.clone()];
        // 只合并已经到达的短信，不为了凑批次额外等待；跨模组不混合。
        for (key, message) in self.queue.iter().skip(1).take(MAX_PUSH_BATCH - 1) {
            if message.modem != messages[0].modem {
                break;
            }
            messages.push(message.clone());
            if !client.can_batch(&messages) {
                messages.pop();
                break;
            }
            keys.push(key.clone());
        }
        for key in &keys {
            self.ledger.insert(key.clone(), Delivery::InFlight);
        }
        if journal.save(&self.checkpoint()?).is_err() {
            for key in &keys {
                self.ledger.insert(key.clone(), Delivery::Pending);
            }
            self.next_send = now.saturating_add(Duration::from_secs(60));
            return Err(journal_error());
        }
        for _ in &keys {
            self.queue.pop_front();
        }
        let send_started = Instant::now();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.send_batch(&messages)
        }))
        .unwrap_or(PushOutcome::Unknown);
        let state = match outcome {
            PushOutcome::Accepted(ref reference)
                if !reference.is_empty()
                    && reference.len() <= 64
                    && reference.bytes().all(|b| b.is_ascii_alphanumeric()) =>
            {
                Delivery::Accepted
            }
            PushOutcome::Rejected => Delivery::Rejected,
            _ => Delivery::Unknown,
        };
        for key in keys {
            self.ledger.insert(key, state);
        }
        self.next_send =
            now.saturating_add(send_started.elapsed())
                .saturating_add(Duration::from_secs(if state == Delivery::Accepted {
                    MIN_PUSH_SECONDS
                } else {
                    60
                }));
        journal
            .save(&self.checkpoint()?)
            .map_err(|_| journal_error())?;
        Ok(true)
    }
    pub fn status(&self) -> Value {
        json!({"enabled":self.enabled,"pending":self.queue.len(),
            "accepted":self.ledger.values().filter(|v| **v == Delivery::Accepted).count(),
            "rejected":self.ledger.values().filter(|v| **v == Delivery::Rejected).count(),
            "unknown":self.ledger.values().filter(|v| **v == Delivery::Unknown).count(),
            "historyFull":self.ledger.len() >= MAX_LEDGER})
    }
    pub fn ready_to_dispatch(&self, now: Duration) -> bool {
        self.enabled && !self.queue.is_empty() && now >= self.next_send
    }
    pub fn idle_delay(&self, now: Duration) -> Duration {
        // 空队列休眠，由新缓存或取消唤醒；待发队列按提供商期限唤醒。
        if !self.enabled || self.queue.is_empty() {
            Duration::from_secs(30)
        } else {
            self.next_send
                .saturating_sub(now)
                .max(Duration::from_millis(1))
        }
    }
}

pub struct PushPlus {
    token: String,
    #[cfg(feature = "pushplus-https")]
    agent: ureq::Agent,
}
impl PushPlus {
    pub fn validate_token(token: &str) -> Result<()> {
        if !(16..=128).contains(&token.len())
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(PanelError::new("PUSHPLUS_TOKEN_INVALID", 422));
        }
        Ok(())
    }
    pub fn new(token: String) -> Result<Self> {
        Self::validate_token(&token)?;
        Ok(Self {
            token,
            #[cfg(feature = "pushplus-https")]
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .https_only(true)
                    .max_redirects(0)
                    .proxy(None)
                    .max_idle_connections(1)
                    .max_idle_connections_per_host(1)
                    .max_idle_age(Duration::from_secs(15))
                    .timeout_global(Some(Duration::from_secs(15)))
                    .timeout_connect(Some(Duration::from_secs(5)))
                    .http_status_as_error(false)
                    .build(),
            ),
        })
    }
    pub fn body(&self, message: &Message) -> Result<Vec<u8>> {
        self.body_batch(std::slice::from_ref(message))
    }
    pub fn body_batch(&self, messages: &[Message]) -> Result<Vec<u8>> {
        if messages.is_empty() || messages.len() > MAX_PUSH_BATCH {
            return Err(invalid());
        }
        fn escape(text: &str) -> String {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#39;")
        }
        let mut content = String::new();
        for message in messages {
            message.validate()?;
            if message.direction != Direction::Incoming || message.modem != messages[0].modem {
                return Err(invalid());
            }
            if !content.is_empty() {
                content.push_str("<hr>");
            }
            content.push_str(&format!(
                "<p>发件人：{}</p><p>设备时间：{}</p><pre>{}</pre>",
                escape(&message.contact),
                escape(&message.timestamp),
                escape(&message.content)
            ));
            if content.len() > MAX_PUSH_BODY {
                return Err(invalid());
            }
        }
        let title = if messages.len() == 1 {
            "网络面板 · 新短信".into()
        } else {
            format!("网络面板 · {}条新短信", messages.len())
        };
        let body = serde_json::to_vec(&json!({"token":self.token,"title":title,"channel":"wechat","template":"html", "content":content})).map_err(|_| invalid())?;
        if body.len() > MAX_PUSH_BODY {
            return Err(invalid());
        }
        Ok(body)
    }
    pub fn response(status: u16, raw: &[u8]) -> PushOutcome {
        if raw.len() > 4096 || status != 200 {
            return PushOutcome::Unknown;
        }
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            return PushOutcome::Unknown;
        };
        match value["code"].as_u64() {
            Some(200) => match value["data"].as_str().filter(|s| {
                !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric())
            }) {
                Some(reference) => PushOutcome::Accepted(reference.into()),
                _ => PushOutcome::Unknown,
            },
            Some(_) => PushOutcome::Rejected,
            None => PushOutcome::Unknown,
        }
    }
}

#[cfg(feature = "pushplus-https")]
impl PushClient for PushPlus {
    fn send(&self, message: &Message) -> PushOutcome {
        self.send_batch(std::slice::from_ref(message))
    }
    fn can_batch(&self, messages: &[Message]) -> bool {
        self.body_batch(messages).is_ok()
    }
    fn send_batch(&self, messages: &[Message]) -> PushOutcome {
        let Ok(body) = self.body_batch(messages) else {
            return PushOutcome::Rejected;
        };
        let Ok(mut response) = self
            .agent
            .post(PUSHPLUS_URL)
            .header("Content-Type", "application/json; charset=utf-8")
            .send(body.as_slice())
        else {
            return PushOutcome::Unknown;
        };
        let status = response.status().as_u16();
        let Ok(raw) = response.body_mut().with_config().limit(4096).read_to_vec() else {
            return PushOutcome::Unknown;
        };
        Self::response(status, &raw)
    }
}

#[cfg(unix)]
pub struct DiskJournal {
    directory: std::path::PathBuf,
}
#[cfg(unix)]
impl DiskJournal {
    pub fn open(directory: &std::path::Path) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::symlink_metadata(directory).map_err(|_| journal_error())?;
        if !meta.is_dir()
            || meta.uid() != 0
            || meta.mode() & 0o077 != 0
            || std::fs::canonicalize(directory).map_err(|_| journal_error())? != directory
        {
            return Err(journal_error());
        }
        Ok(Self {
            directory: directory.into(),
        })
    }
    pub fn load(&self) -> Result<Option<Forwarder>> {
        use std::{io::Read, os::unix::fs::MetadataExt};
        let path = self.directory.join("sms-forward.json");
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(journal_error()),
        };
        if !meta.is_file()
            || meta.uid() != 0
            || meta.mode() & 0o077 != 0
            || meta.nlink() != 1
            || meta.len() > MAX_JOURNAL_BYTES as u64
        {
            return Err(journal_error());
        }
        let mut raw = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| journal_error())?
            .take(MAX_JOURNAL_BYTES as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| journal_error())?;
        Forwarder::restore(&raw).map(Some)
    }
}
#[cfg(unix)]
impl Journal for DiskJournal {
    fn save(&mut self, checkpoint: &[u8]) -> Result<()> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        Self::open(&self.directory)?;
        Forwarder::restore(checkpoint)?;
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| journal_error())?;
        let name: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let temporary = self.directory.join(format!(".sms-forward-{name}.tmp"));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| journal_error())?;
        let outcome = (|| {
            file.write_all(checkpoint).map_err(|_| journal_error())?;
            file.sync_all().map_err(|_| journal_error())?;
            std::fs::rename(&temporary, self.directory.join("sms-forward.json"))
                .map_err(|_| journal_error())?;
            std::fs::File::open(&self.directory)
                .and_then(|f| f.sync_all())
                .map_err(|_| journal_error())
        })();
        drop(file);
        if outcome.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        outcome
    }
}
