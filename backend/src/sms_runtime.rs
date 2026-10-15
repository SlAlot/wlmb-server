//! 接收与转发共用缓存；关闭任务释放监听和HTTPS资源，未知结果不自动重试。
use crate::{
    PanelError, Result,
    sms::{Direction, MAX_SNAPSHOT_BYTES},
    sms_forward::{Forwarder, Journal, PushClient, PushPlus},
    sms_pdu::Record,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
fn busy() -> PanelError {
    PanelError::new("SMS_BUSY", 409)
}
fn invalid() -> PanelError {
    PanelError::new("SMS_FORWARD_INVALID", 422)
}
pub const MIN_POLL_SECONDS: u64 = 3;
pub const DEFAULT_POLL_SECONDS: u64 = 10;
pub const MIN_FALLBACK_SECONDS: u64 = 5;
const BUSY_RETRY_SECONDS: u64 = 5;
pub fn forwardable(records: &[Record]) -> Vec<crate::sms::Message> {
    records
        .iter()
        .filter(|r| r.concatenation.is_none())
        .map(|r| r.message.clone())
        .collect()
}
pub trait SmsDevice: Send + Sync {
    fn stop_notices(&self) {}
    fn notice_idle_delay(&self) -> Duration {
        if self.notice_supported() {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(60)
        }
    }
    fn poll_notice(&self, _modem: &str) -> Result<()> {
        Ok(())
    }
    fn notice_supported(&self) -> bool {
        false
    }
    fn notice_generation(&self, _modem: &str) -> u64 {
        0
    }
    fn writable(&self) -> bool {
        true
    }
    fn writable_for(&self, _modem: &str) -> bool {
        self.writable()
    }
    fn writable_snapshot(&self, row: &Value) -> bool {
        self.writable_for(row["id"].as_str().unwrap_or(""))
    }
    fn source_info(&self, _modem: &str) -> Value {
        json!({"source":"modem-at","deleteScope":"modem-storage"})
    }
    fn revision(&self, modem: &str) -> Result<String>;
    fn read(&self, modem: &str) -> Result<Vec<Record>>;
    fn read_event(&self, modem: &str) -> Result<Vec<Record>> {
        self.read(modem)
    }
    fn send(&self, modem: &str, number: &str, content: &str) -> Result<Value>;
    fn delete(&self, _modem: &str, _fingerprint: &str) -> Result<Value> {
        Err(PanelError::new("SMS_DELETE_UNSUPPORTED", 501))
    }
}
struct Cached {
    records: Vec<Record>,
    revision: String,
    finished: Instant,
    observed: u64,
    forward_scan: bool,
}
pub struct SmsHub {
    pub device: Arc<dyn SmsDevice>,
    cache: Mutex<BTreeMap<String, Cached>>,
    scanning: Mutex<()>,
    attempts: Mutex<BTreeMap<String, Instant>>,
    notices: Mutex<BTreeMap<String, u64>>,
    read_state: Mutex<crate::sms_read_state::Ledger>,
}
impl SmsHub {
    pub fn new(device: Arc<dyn SmsDevice>) -> Arc<Self> {
        Arc::new(Self {
            device,
            cache: Mutex::new(BTreeMap::new()),
            scanning: Mutex::new(()),
            attempts: Mutex::new(BTreeMap::new()),
            notices: Mutex::new(BTreeMap::new()),
            read_state: Mutex::new(crate::sms_read_state::Ledger::memory()),
        })
    }
    pub fn with_read_store(
        device: Arc<dyn SmsDevice>,
        store: Arc<dyn crate::sms_read_state::ReadStore>,
    ) -> Result<Arc<Self>> {
        let ledger = crate::sms_read_state::Ledger::open(store)?;
        Ok(Arc::new(Self {
            device,
            cache: Mutex::new(BTreeMap::new()),
            scanning: Mutex::new(()),
            attempts: Mutex::new(BTreeMap::new()),
            notices: Mutex::new(BTreeMap::new()),
            read_state: Mutex::new(ledger),
        }))
    }
    pub fn refresh(&self, modem: &str) -> Result<Vec<Record>> {
        self.scan(modem, false, Duration::from_secs(30), false)
    }
    pub fn refresh_fresh(&self, modem: &str) -> Result<Vec<Record>> {
        self.scan(modem, true, Duration::from_secs(30), false)
    }
    pub fn refresh_forward(&self, modem: &str, fresh: bool, seconds: u64) -> Result<Vec<Record>> {
        if !(MIN_POLL_SECONDS..=300).contains(&seconds) {
            return Err(invalid());
        }
        self.scan(modem, fresh, Duration::from_secs(seconds), true)
    }
    fn records(
        &self,
        modem: &str,
        consumed: Option<Instant>,
    ) -> Result<Option<(Instant, String, Vec<Record>)>> {
        Ok(self
            .cache
            .lock()
            .map_err(|_| busy())?
            .get(modem)
            .filter(|r| consumed != Some(r.finished))
            .map(|r| (r.finished, r.revision.clone(), r.records.clone())))
    }
    fn scan(
        &self,
        modem: &str,
        fresh: bool,
        interval: Duration,
        forward: bool,
    ) -> Result<Vec<Record>> {
        valid_modem(modem)?;
        let _guard = self.scanning.try_lock().map_err(|_| busy())?;
        let revision = self.device.revision(modem)?;
        let notice = self.device.notice_generation(modem);
        let consumed = self
            .notices
            .lock()
            .map_err(|_| busy())?
            .get(modem)
            .copied()
            .unwrap_or(0);
        let fast = self
            .attempts
            .lock()
            .map_err(|_| busy())?
            .get(modem)
            .is_some_and(|at| {
                crate::sms_notice::fast_scan_allowed(notice, consumed, *at, Instant::now())
            });
        {
            let cache = self.cache.lock().map_err(|_| busy())?;
            if let Some(row) = cache.get(modem).filter(|row| {
                (!fresh || !forward && row.forward_scan) && !fast && row.revision == revision
            }) {
                if row.finished.elapsed() < interval {
                    return Ok(row.records.clone());
                }
            }
        }
        {
            let mut attempts = self.attempts.lock().map_err(|_| busy())?;
            if attempts
                .get(modem)
                .is_some_and(|at| at.elapsed() < interval && !fast)
            {
                return Err(PanelError::new("SMS_REFRESH_COOLDOWN", 409));
            }
            if !attempts.contains_key(modem) && attempts.len() >= 16 {
                return Err(busy());
            }
            attempts.insert(modem.into(), Instant::now());
        }
        self.notices
            .lock()
            .map_err(|_| busy())?
            .insert(modem.into(), notice);
        let read = if fast {
            self.device.read_event(modem)
        } else {
            self.device.read(modem)
        };
        if let Ok(mut attempts) = self.attempts.lock() {
            attempts.insert(modem.into(), Instant::now());
        }
        let records = read?;
        if self.device.revision(modem)? != revision {
            self.invalidate(modem);
            return Err(PanelError::new("MODEM_CHANGED", 409));
        }
        if records.len() > crate::sms::MAX_MESSAGES {
            return Err(invalid());
        }
        let mut bytes = 0;
        for r in &records {
            r.message.validate()?;
            if r.message.modem != modem {
                return Err(invalid());
            }
            bytes += r.message.content.len() + r.message.contact.len() + r.message.timestamp.len();
        }
        if bytes > MAX_SNAPSHOT_BYTES {
            return Err(invalid());
        }
        self.read_state.lock().map_err(|_| busy())?.reconcile(
            &revision,
            &records
                .iter()
                .map(|r| r.message.clone())
                .collect::<Vec<_>>(),
        )?;
        let mut cache = self.cache.lock().map_err(|_| busy())?;
        if !cache.contains_key(modem) && cache.len() >= 16 {
            return Err(busy());
        }
        cache.insert(
            modem.into(),
            Cached {
                records: records.clone(),
                revision,
                finished: Instant::now(),
                observed: crate::events::wall_ms(),
                forward_scan: forward,
            },
        );
        Ok(records)
    }
    pub fn cached(&self, modem: &str) -> Result<Value> {
        valid_modem(modem)?;
        let revision = self.device.revision(modem)?;
        let mut cache = self.cache.lock().map_err(|_| busy())?;
        if cache.get(modem).is_some_and(|row| row.revision != revision) {
            cache.remove(modem);
        }
        let Some(row) = cache.get(modem) else {
            return Ok(json!({"messages":[],"loaded":false,"stale":true}));
        };
        let ledger = self.read_state.lock().map_err(|_| busy())?;
        let messages: Result<Vec<Value>>=row.records.iter().map(|r| Ok(json!({"id":r.message.fingerprint()?,"modem":r.message.modem,"storage":r.message.storage,"index":r.message.index,"partCount":r.indexes.len(),"complete":r.concatenation.is_none(),"pendingPart":r.concatenation.as_ref().map(|p|p.part),"expectedParts":r.concatenation.as_ref().map(|p|p.total),"direction":match r.message.direction { Direction::Incoming=>"incoming",Direction::Outgoing=>"outgoing",Direction::Unknown=>"unknown" },"phone":r.message.contact,"content":r.message.content,"timestamp":r.message.timestamp,"unread":ledger.unread(&revision,&r.message)?,"readStateSource":"plugin"}))).collect();
        Ok(
            json!({"messages":messages?,"loaded":true,"backend":self.device.source_info(modem),"observedAtMs":row.observed,"stale":row.finished.elapsed()>Duration::from_secs(180),"minRefreshSeconds":30,"readStatePersistent":ledger.persistent(),"deviceUnreadMayChange":true}),
        )
    }
    pub fn mark_read(&self, modem: &str, fingerprint: &str, unread: bool) -> Result<Value> {
        let _scan = self.scanning.try_lock().map_err(|_| busy())?;
        let revision = self.device.revision(modem)?;
        let cache = self.cache.lock().map_err(|_| busy())?;
        let row = cache
            .get(modem)
            .filter(|r| r.revision == revision && r.finished.elapsed() <= Duration::from_secs(180))
            .ok_or_else(|| PanelError::new("SMS_CHANGED", 409))?;
        let message = row
            .records
            .iter()
            .find(|r| r.message.fingerprint().is_ok_and(|id| id == fingerprint))
            .ok_or_else(|| PanelError::new("SMS_CHANGED", 409))?;
        self.read_state
            .lock()
            .map_err(|_| busy())?
            .mark(&revision, &message.message, unread)?;
        Ok(json!({"saved":true,"id":fingerprint,"unread":unread,"readStateSource":"plugin"}))
    }
    pub fn invalidate(&self, modem: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(modem);
        }
    }
    pub fn delete(&self, modem: &str, fingerprint: &str) -> Result<Value> {
        let _guard = self.scanning.try_lock().map_err(|_| busy())?;
        self.invalidate(modem);
        self.device.delete(modem, fingerprint)
    }
    pub fn send(&self, modem: &str, number: &str, content: &str) -> Result<Value> {
        let _guard = self.scanning.try_lock().map_err(|_| busy())?;
        self.device.send(modem, number, content)
    }
}
fn valid_modem(modem: &str) -> Result<()> {
    if modem.is_empty() || modem.len() > 128 || modem.chars().any(char::is_control) {
        Err(invalid())
    } else {
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub receive_enabled: bool,
    pub modem: String,
    pub poll_seconds: u64,
    token: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            receive_enabled: false,
            modem: String::new(),
            poll_seconds: DEFAULT_POLL_SECONDS,
            token: String::new(),
        }
    }
}
impl Settings {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(invalid());
        }
        let v: Value = serde_json::from_slice(raw).map_err(|_| invalid())?;
        if v["version"] != 1 {
            return Err(invalid());
        }
        Self::default().updated(&v)
    }
    fn updated(&self, v: &Value) -> Result<Self> {
        if !v.is_object() {
            return Err(invalid());
        }
        let enabled = v["enabled"].as_bool().ok_or_else(invalid)?;
        let receive_enabled = v
            .get("receiveEnabled")
            .map(|v| v.as_bool().ok_or_else(invalid))
            .transpose()?
            .unwrap_or(self.receive_enabled);
        let modem = v["modem"].as_str().ok_or_else(invalid)?.to_owned();
        let poll_seconds = v["pollSeconds"]
            .as_u64()
            .filter(|s| (MIN_POLL_SECONDS..=300).contains(s))
            .ok_or_else(invalid)?;
        let mut token = self.token.clone();
        if let Some(next) = v.get("token") {
            let next = next.as_str().ok_or_else(invalid)?;
            if !next.is_empty() {
                PushPlus::validate_token(next)?;
                token = next.into();
            }
        }
        if v["clearToken"] == true {
            if enabled {
                return Err(invalid());
            }
            token.clear();
        }
        if !modem.is_empty() {
            valid_modem(&modem)?;
        }
        if enabled || receive_enabled {
            valid_modem(&modem)?;
        }
        if enabled {
            PushPlus::validate_token(&token)?;
        }
        Ok(Self {
            enabled,
            receive_enabled,
            modem,
            poll_seconds,
            token,
        })
    }
    pub fn public(&self) -> Value {
        json!({"enabled":self.enabled,"receiveEnabled":self.receive_enabled,"modem":self.modem,"pollSeconds":self.poll_seconds.max(MIN_FALLBACK_SECONDS),"tokenSet":!self.token.is_empty(),"provider":"pushplus","historyPolicy":"new-after-enable","minimumPollSeconds":MIN_FALLBACK_SECONDS,"defaultPollSeconds":DEFAULT_POLL_SECONDS,"minimumPushSeconds":crate::sms_forward::MIN_PUSH_SECONDS,"receiveStrategy":"event-first","eventDeliveryGuaranteed":false,"pollTiming":"fallback-after-scan-completion","legacyIntervalClamped":self.poll_seconds<MIN_FALLBACK_SECONDS})
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&json!({"version":1,"enabled":self.enabled,"receiveEnabled":self.receive_enabled,"modem":self.modem,"pollSeconds":self.poll_seconds,"token":self.token})).map_err(|_| invalid())
    }
}
pub trait Store: Send + Sync {
    fn load(&self) -> Result<Settings>;
    fn save(&self, settings: &Settings) -> Result<()>;
}
pub struct Resources {
    pub forwarder: Forwarder,
    pub journal: Box<dyn Journal + Send>,
    pub client: Box<dyn PushClient + Send>,
}
pub trait Factory: Send + Sync {
    fn preflight(&self, token: &str) -> Result<()> {
        drop(self.create(token)?);
        Ok(())
    }
    fn create(&self, token: &str) -> Result<Resources>;
}
#[derive(Default)]
struct Signal {
    stop: Mutex<(bool, u64)>,
    changed: Condvar,
    ready: AtomicBool,
}
impl Signal {
    fn stopped(&self) -> bool {
        self.stop.lock().map_or(true, |s| s.0)
    }
    fn cancel(&self) {
        if let Ok(mut s) = self.stop.lock() {
            s.0 = true;
            self.changed.notify_all();
        }
    }
    fn wait(&self, delay: Duration) {
        self.wait_since(self.generation(), delay);
    }
    fn generation(&self) -> u64 {
        self.stop.lock().map_or(0, |s| s.1)
    }
    fn wait_since(&self, generation: u64, delay: Duration) {
        // 谓词与通知共用锁，避免通知发生在检查和休眠之间而丢失。
        if let Ok(s) = self.stop.lock() {
            drop(
                self.changed
                    .wait_timeout_while(s, delay, |s| !s.0 && s.1 == generation),
            );
        }
    }
    fn notify(&self) {
        if let Ok(mut s) = self.stop.lock() {
            s.1 = s.1.wrapping_add(1);
            self.changed.notify_all();
        }
    }
}
struct Worker {
    signal: Arc<Signal>,
    thread: JoinHandle<()>,
    forwarding: Option<JoinHandle<()>>,
}
#[test]
fn notification_before_wait_is_not_lost_and_cancel_does_not_wait_idle_deadline() {
    let signal = Signal::default();
    let generation = signal.generation();
    signal.notify();
    let started = Instant::now();
    signal.wait_since(generation, Duration::from_secs(30));
    assert!(started.elapsed() < Duration::from_secs(1));
    signal.cancel();
    signal.wait(Duration::from_secs(30));
    assert!(signal.stopped());
}
struct State {
    settings: Settings,
    worker: Option<Worker>,
}
pub struct ForwardRuntime {
    hub: Arc<SmsHub>,
    store: Arc<dyn Store>,
    factory: Arc<dyn Factory>,
    state: Mutex<State>,
    status: Arc<Mutex<Value>>,
}
impl ForwardRuntime {
    pub fn new(
        hub: Arc<SmsHub>,
        store: Arc<dyn Store>,
        factory: Arc<dyn Factory>,
    ) -> Result<Arc<Self>> {
        let settings = store.load()?;
        Ok(Arc::new(Self {
            hub,
            store,
            factory,
            state: Mutex::new(State {
                settings,
                worker: None,
            }),
            status: Arc::new(Mutex::new(json!({"running":false,"state":"disabled"}))),
        }))
    }
    pub fn public(&self) -> Result<Value> {
        let state = self.state.try_lock().map_err(|_| busy())?;
        let mut v = state.settings.public();
        v["runtime"] = self.status.lock().map_err(|_| busy())?.clone();
        if state.worker.as_ref().is_some_and(|w| {
            w.thread.is_finished() || w.forwarding.as_ref().is_some_and(|t| t.is_finished())
        }) && (state.settings.enabled || state.settings.receive_enabled)
        {
            v["runtime"] =
                json!({"running":false,"state":"failed","error":"SMS_FORWARD_WORKER_STOPPED"});
        }
        Ok(v)
    }
    pub fn start_saved(&self) -> Result<()> {
        let state = self.state.lock().map_err(|_| busy())?;
        if !state.settings.enabled && !state.settings.receive_enabled {
            return Ok(());
        }
        let v = state.settings.public();
        drop(state);
        self.update(&v)
            .inspect_err(|e| {
                if let Ok(mut s) = self.status.lock() {
                    *s = json!({"running":false,"state":"failed","error":e.code});
                }
            })
            .map(|_| ())
    }
    pub fn update(&self, body: &Value) -> Result<Value> {
        let mut state = self.state.try_lock().map_err(|_| busy())?;
        let next = state.settings.updated(body)?;
        if next == state.settings
            && state.worker.as_ref().is_some_and(|w| {
                !w.thread.is_finished() && w.forwarding.as_ref().is_none_or(|t| !t.is_finished())
            })
        {
            let mut v = next.public();
            v["runtime"] = self.status.lock().map_err(|_| busy())?.clone();
            return Ok(v);
        }
        if next.enabled {
            self.factory.preflight(&next.token)?;
        }
        self.store.save(&next)?;
        state.settings = next.clone();
        if let Some(worker) = state.worker.take() {
            worker.signal.cancel();
            if let Ok(mut s) = self.status.lock() {
                *s = json!({"running":true,"state":"stopping"});
            }
            if worker.thread.join().is_err() {
                if let Ok(mut s) = self.status.lock() {
                    *s =
                        json!({"running":false,"state":"failed","error":"SMS_FORWARD_STOP_FAILED"});
                }
                return Err(PanelError::new("SMS_FORWARD_STOP_FAILED", 503));
            }
            if let Some(t) = worker.forwarding {
                let _ = t.join();
            }
        }
        if next.enabled || next.receive_enabled {
            let resources = if next.enabled {
                Some(self.factory.create(&next.token).inspect_err(|e| {
                    if let Ok(mut s) = self.status.lock() {
                        *s = json!({"running":false,"state":"failed","error":e.code});
                    }
                })?)
            } else {
                None
            };
            let signal = Arc::new(Signal::default());
            let worker_signal = signal.clone();
            let hub = self.hub.clone();
            let status = self.status.clone();
            let receive_settings = next.clone();
            let thread = std::thread::Builder::new()
                .name("wlmb-sms-receive".into())
                .stack_size(256 * 1024)
                .spawn(move || run_receive(hub, receive_settings, worker_signal, status))
                .map_err(|_| {
                    if let Ok(mut s) = self.status.lock() {
                        *s = json!({"running":false,"state":"failed","error":"SMS_FORWARD_START_FAILED"});
                    }
                    PanelError::new("SMS_FORWARD_START_FAILED", 503)
                })?;
            let forwarding = if let Some(resources) = resources {
                let hub = self.hub.clone();
                let worker_signal = signal.clone();
                let status = self.status.clone();
                match std::thread::Builder::new()
                    .name("wlmb-sms-forward".into())
                    .stack_size(256 * 1024)
                    .spawn(move || run_worker(hub, next, resources, worker_signal, status))
                {
                    Ok(t) => Some(t),
                    Err(_) => {
                        signal.cancel();
                        let _ = thread.join();
                        return Err(PanelError::new("SMS_FORWARD_START_FAILED", 503));
                    }
                }
            } else {
                None
            };
            state.worker = Some(Worker {
                signal,
                thread,
                forwarding,
            });
        } else if let Ok(mut status) = self.status.lock() {
            *status = json!({"running":false,"state":"disabled","receiveRunning":false,"forwardRunning":false,"eventConnected":false});
        }
        let mut result = state.settings.public();
        result["runtime"] = self.status.lock().map_err(|_| busy())?.clone();
        Ok(result)
    }
    pub fn shutdown(&self) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(w) = state.worker.take() {
                w.signal.cancel();
                let _ = w.thread.join();
                if let Some(t) = w.forwarding {
                    let _ = t.join();
                }
            }
        }
        status_merge(
            &self.status,
            json!({"running":false,"state":"disabled","receiveRunning":false,"forwardRunning":false,"eventConnected":false}),
        );
    }
}
impl Drop for ForwardRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn status_merge(status: &Mutex<Value>, value: Value) {
    if let (Ok(mut current), Some(fields)) = (status.lock(), value.as_object()) {
        for (key, value) in fields {
            current[key] = value.clone();
        }
    }
}
fn run_receive(
    hub: Arc<SmsHub>,
    settings: Settings,
    signal: Arc<Signal>,
    status: Arc<Mutex<Value>>,
) {
    // 有事件先收取，无事件才到期兜底；忙碌退避，不与HTTPS任务互相占锁。
    let fallback = settings.poll_seconds.max(MIN_FALLBACK_SECONDS);
    let mut due = Instant::now();
    let mut last = Instant::now();
    let mut seen = 0;
    let mut failures = 0u32;
    let mut revision = String::new();
    let mut first = true;
    status_merge(
        &status,
        json!({"running":true,"receiveRunning":true,"receiveState":"starting","state":"starting"}),
    );
    while !signal.stopped() {
        let _ = hub.device.poll_notice(&settings.modem);
        if signal.stopped() {
            break;
        }
        let notice = hub.device.notice_generation(&settings.modem);
        if failures == 0 && crate::sms_notice::fast_scan_allowed(notice, seen, last, Instant::now())
        {
            due = Instant::now();
        }
        if Instant::now() >= due {
            seen = notice;
            let result: Result<()> = (|| {
                let current = hub.device.revision(&settings.modem)?;
                if revision != current && !revision.is_empty() {
                    first = true;
                    signal.ready.store(false, Ordering::Release);
                    hub.invalidate(&settings.modem);
                }
                // 只读失败按短间隔恢复；成功快照仍按原兜底间隔复用。
                if first || failures > 0 {
                    hub.scan(&settings.modem, true, Duration::from_secs(2), true)?;
                } else {
                    hub.refresh_forward(&settings.modem, false, fallback)?;
                }
                revision = current;
                Ok(())
            })();
            last = Instant::now();
            match result {
                Ok(()) => {
                    first = false;
                    signal.ready.store(true, Ordering::Release);
                    failures = 0;
                    due = Instant::now() + Duration::from_secs(fallback);
                    status_merge(
                        &status,
                        json!({"receiveRunning":true,"receiveState":"running","lastScanAtMs":crate::events::wall_ms(),
                        "eventConnected":hub.device.notice_supported(),"fallbackSeconds":fallback,"receiveStrategy":"event-first","receiveError":null}),
                    );
                    if !settings.enabled {
                        status_merge(&status, json!({"running":true,"state":"running"}));
                    }
                    signal.notify();
                }
                Err(e) => {
                    failures = failures.saturating_add(1).min(4);
                    let retry = if first && e.code == "SMS_REFRESH_COOLDOWN" {
                        2
                    } else if matches!(
                        e.code,
                        "AT_BUSY"
                            | "SMS_BUSY"
                            | "SMS_SHARED_BUSY"
                            | "SMS_REFRESH_COOLDOWN"
                            | "SYSTEM_BUSY"
                            | "AT_COOLDOWN"
                    ) {
                        BUSY_RETRY_SECONDS
                    } else {
                        (30u64 << failures).min(300)
                    };
                    due = Instant::now() + Duration::from_secs(retry);
                    status_merge(
                        &status,
                        json!({"receiveState":"waiting","receiveError":e.code,"retrySeconds":retry}),
                    );
                    if !settings.enabled || first {
                        status_merge(&status, json!({"state":"waiting"}));
                    }
                }
            }
        }
        // 无事件通道直接等到下个截止点；突发通知只等待必要的合并预算。
        let wake = if failures == 0 && notice > seen {
            due.min(last + Duration::from_secs(2))
        } else {
            due
        };
        signal.wait(
            hub.device
                .notice_idle_delay()
                .min(wake.saturating_duration_since(Instant::now()))
                .max(Duration::from_millis(10)),
        );
    }
    hub.device.stop_notices();
    status_merge(
        &status,
        json!({"receiveRunning":false,"receiveState":"disabled","eventConnected":false}),
    );
}
fn run_worker(
    hub: Arc<SmsHub>,
    settings: Settings,
    mut resources: Resources,
    signal: Arc<Signal>,
    status: Arc<Mutex<Value>>,
) {
    // 只复制更新快照；空闲等待通知，投递前再次检查当前模组身份。
    let born = Instant::now();
    let mut first = true;
    let mut revision = None;
    let mut consumed = None;
    resources.forwarder.set_enabled(false);
    status_merge(&status, json!({"running":true,"forwardRunning":true}));
    while !signal.stopped() {
        let generation = signal.generation();
        if signal.ready.load(Ordering::Acquire) {
            if let Ok(Some((stamp, current_revision, records))) =
                hub.records(&settings.modem, consumed)
            {
                if consumed != Some(stamp) {
                    consumed = Some(stamp);
                    let poll: Result<()> = (|| {
                        if revision.as_ref() != Some(&current_revision) {
                            resources.forwarder.set_enabled(false);
                            first = true;
                            revision = Some(current_revision);
                        }
                        if signal.stopped() {
                            return Ok(());
                        }
                        let before = resources.forwarder.checkpoint()?;
                        let messages = forwardable(&records);
                        resources.forwarder.observe(&settings.modem, &messages)?;
                        if first {
                            resources.forwarder.set_enabled(true);
                            first = false;
                        }
                        let checkpoint = resources.forwarder.checkpoint()?;
                        if checkpoint != before {
                            resources.journal.save(&checkpoint)?;
                        }
                        Ok(())
                    })();
                    match poll {
                        Ok(()) => {
                            status_merge(
                                &status,
                                json!({"running":true,"state":"running","counts":resources.forwarder.status()}),
                            );
                        }
                        Err(e) => {
                            status_merge(
                                &status,
                                json!({"running":true,"state":"waiting","error":e.code,"counts":resources.forwarder.status()}),
                            );
                        }
                    }
                }
            }
        }
        if signal.stopped() {
            break;
        }
        let mut dispatch_wait = None;
        if !first && resources.forwarder.ready_to_dispatch(born.elapsed()) {
            match hub.device.revision(&settings.modem) {
                Ok(current) if revision.as_ref() == Some(&current) => {}
                Ok(_) => {
                    resources.forwarder.set_enabled(false);
                    first = true;
                    hub.invalidate(&settings.modem);
                }
                Err(e) => {
                    dispatch_wait = Some(Duration::from_secs(60));
                    status_merge(&status, json!({"state":"waiting","error":e.code}));
                }
            }
        }
        if !first && dispatch_wait.is_none() {
            if let Err(e) = resources.forwarder.dispatch(
                born.elapsed(),
                resources.journal.as_mut(),
                resources.client.as_ref(),
            ) {
                if let Ok(mut s) = status.lock() {
                    s["error"] = json!(e.code);
                }
            }
            if let Ok(mut s) = status.lock() {
                s["counts"] = resources.forwarder.status();
            }
        }
        signal.wait_since(
            generation,
            dispatch_wait.unwrap_or_else(|| resources.forwarder.idle_delay(born.elapsed())),
        );
    }
    resources.forwarder.set_enabled(false);
    status_merge(
        &status,
        json!({"running":false,"state":"disabled","forwardRunning":false}),
    );
}

#[cfg(unix)]
pub struct DiskStore {
    directory: std::path::PathBuf,
}
#[cfg(unix)]
impl DiskStore {
    pub fn open(directory: &std::path::Path) -> Result<Self> {
        crate::sms_forward::DiskJournal::open(directory)?;
        Ok(Self {
            directory: directory.into(),
        })
    }
}
#[cfg(unix)]
impl Store for DiskStore {
    fn load(&self) -> Result<Settings> {
        use std::{io::Read, os::unix::fs::MetadataExt};
        let path = self.directory.join("sms-settings.json");
        let before = match std::fs::symlink_metadata(&path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(_) => return Err(invalid()),
        };
        if !before.is_file()
            || before.uid() != 0
            || before.mode() & 0o077 != 0
            || before.nlink() != 1
            || before.len() > 2048
        {
            return Err(invalid());
        }
        let file = std::fs::File::open(&path).map_err(|_| invalid())?;
        let after = file.metadata().map_err(|_| invalid())?;
        if before.ino() != after.ino() || before.dev() != after.dev() {
            return Err(invalid());
        }
        let mut raw = vec![];
        file.take(2049)
            .read_to_end(&mut raw)
            .map_err(|_| invalid())?;
        Settings::decode(&raw)
    }
    fn save(&self, settings: &Settings) -> Result<()> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        Self::open(&self.directory)?;
        let raw = settings.encode()?;
        Settings::decode(&raw)?;
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| invalid())?;
        let name: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let path = self.directory.join(format!(".sms-settings-{name}.tmp"));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|_| invalid())?;
        let result = (|| {
            file.write_all(&raw).map_err(|_| invalid())?;
            file.sync_all().map_err(|_| invalid())?;
            std::fs::rename(&path, self.directory.join("sms-settings.json"))
                .map_err(|_| invalid())?;
            std::fs::File::open(&self.directory)
                .and_then(|f| f.sync_all())
                .map_err(|_| invalid())
        })();
        drop(file);
        if result.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        result
    }
}
#[cfg(unix)]
pub struct NativeFactory {
    pub directory: std::path::PathBuf,
}
#[cfg(unix)]
impl Factory for NativeFactory {
    fn preflight(&self, token: &str) -> Result<()> {
        PushPlus::validate_token(token)?;
        if !cfg!(feature = "pushplus-https") {
            return Err(PanelError::new("PUSHPLUS_TLS_NOT_BUILT", 501));
        }
        let journal = crate::sms_forward::DiskJournal::open(&self.directory)?;
        drop(journal.load()?);
        Ok(())
    }
    fn create(&self, token: &str) -> Result<Resources> {
        #[cfg(not(feature = "pushplus-https"))]
        {
            let _ = token;
            Err(PanelError::new("PUSHPLUS_TLS_NOT_BUILT", 501))
        }
        #[cfg(feature = "pushplus-https")]
        {
            let journal = crate::sms_forward::DiskJournal::open(&self.directory)?;
            let forwarder = journal.load()?.unwrap_or_default();
            let client = PushPlus::new(token.into())?;
            Ok(Resources {
                forwarder,
                journal: Box::new(journal),
                client: Box::new(client),
            })
        }
    }
}
