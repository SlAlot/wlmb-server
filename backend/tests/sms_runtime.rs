#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use wlmb_server::{
    PanelError, Result,
    sms_forward::{Forwarder, Journal, PushClient, PushOutcome},
    sms_pdu::{Record, decode, encode_submit},
    sms_runtime::{Factory, ForwardRuntime, Resources, Settings, SmsDevice, SmsHub, Store},
};
struct Device {
    reads: AtomicUsize,
    fail: AtomicBool,
    revision: AtomicUsize,
}
struct Notified {
    reads: AtomicUsize,
    notice: AtomicUsize,
}
impl SmsDevice for Notified {
    fn notice_supported(&self) -> bool {
        true
    }
    fn notice_generation(&self, _: &str) -> u64 {
        self.notice.load(Ordering::SeqCst) as u64
    }
    fn revision(&self, _: &str) -> Result<String> {
        Ok("same-instance".into())
    }
    fn read(&self, _: &str) -> Result<Vec<Record>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    fn send(&self, _: &str, _: &str, _: &str) -> Result<Value> {
        panic!("No send in receive test")
    }
}
#[test]
fn new_notice_bypasses_only_the_normal_cooldown_not_the_fast_budget() {
    let device = Arc::new(Notified {
        reads: AtomicUsize::new(0),
        notice: AtomicUsize::new(0),
    });
    let hub = SmsHub::new(device.clone());
    hub.refresh("m1").unwrap();
    device.notice.store(1, Ordering::SeqCst);
    hub.refresh("m1").unwrap();
    assert_eq!(device.reads.load(Ordering::SeqCst), 1);
    std::thread::sleep(Duration::from_millis(2050));
    hub.refresh("m1").unwrap();
    assert_eq!(device.reads.load(Ordering::SeqCst), 2);
    hub.refresh("m1").unwrap();
    assert_eq!(device.reads.load(Ordering::SeqCst), 2);
}
impl SmsDevice for Device {
    fn revision(&self, _: &str) -> Result<String> {
        Ok(self.revision.load(Ordering::SeqCst).to_string())
    }
    fn read(&self, id: &str) -> Result<Vec<Record>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(PanelError::new("AT_BUSY", 409));
        }
        let pdu = encode_submit("10086", "old secret body")?;
        let mut row = decode(&pdu.hex, pdu.tpdu_bytes, id, "ME", 1, 3)?;
        row.message.direction = wlmb_server::sms::Direction::Incoming;
        Ok(vec![row])
    }
    fn send(&self, _: &str, _: &str, _: &str) -> Result<Value> {
        Ok(json!({"sent":true}))
    }
}
struct Memory {
    raw: Mutex<Vec<u8>>,
    fail: AtomicBool,
}
impl Store for Memory {
    fn load(&self) -> Result<Settings> {
        Settings::decode(&self.raw.lock().unwrap())
    }
    fn save(&self, s: &Settings) -> Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(PanelError::new("STORE_FAILED", 503));
        }
        *self.raw.lock().unwrap() = s.encode()?;
        Ok(())
    }
}
struct Log {
    raw: Arc<Mutex<Vec<u8>>>,
}
impl Journal for Log {
    fn save(&mut self, b: &[u8]) -> Result<()> {
        *self.raw.lock().unwrap() = b.into();
        Ok(())
    }
}
struct Client {
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}
impl PushClient for Client {
    fn send(&self, _: &wlmb_server::sms::Message) -> PushOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        PushOutcome::Accepted("ok1".into())
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
struct Build {
    creates: AtomicUsize,
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    raw: Arc<Mutex<Vec<u8>>>,
}
impl Factory for Build {
    fn create(&self, _: &str) -> Result<Resources> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        let raw = self.raw.lock().unwrap();
        let forwarder = if raw.is_empty() {
            Forwarder::default()
        } else {
            Forwarder::restore(&raw)?
        };
        Ok(Resources {
            forwarder,
            journal: Box::new(Log {
                raw: self.raw.clone(),
            }),
            client: Box::new(Client {
                calls: self.calls.clone(),
                drops: self.drops.clone(),
            }),
        })
    }
}
type Setup = (
    Arc<Device>,
    Arc<Memory>,
    Arc<Build>,
    Arc<SmsHub>,
    Arc<ForwardRuntime>,
);
fn setup() -> Setup {
    let d = Arc::new(Device {
        reads: AtomicUsize::new(0),
        fail: AtomicBool::new(false),
        revision: AtomicUsize::new(1),
    });
    let store = Arc::new(Memory {
        raw: Mutex::new(Settings::default().encode().unwrap()),
        fail: AtomicBool::new(false),
    });
    let build = Arc::new(Build {
        creates: AtomicUsize::new(0),
        calls: Arc::new(AtomicUsize::new(0)),
        drops: Arc::new(AtomicUsize::new(0)),
        raw: Arc::new(Mutex::new(vec![])),
    });
    let hub = SmsHub::new(d.clone());
    let runtime = ForwardRuntime::new(hub.clone(), store.clone(), build.clone()).unwrap();
    (d, store, build, hub, runtime)
}
fn enabled() -> Value {
    json!({"enabled":true,"modem":"m1","pollSeconds":60,"token":"SecretPushToken12345678"})
}
#[test]
fn event_first_settings_default_to_ten_seconds_and_preserve_saved_intervals() {
    assert_eq!(Settings::default().public()["pollSeconds"], 10);
    assert_eq!(Settings::default().public()["minimumPollSeconds"], 5);
    for seconds in [3, 5, 10, 90, 120, 300] {
        let raw = serde_json::to_vec(
            &json!({"version":1,"enabled":false,"modem":"m1","pollSeconds":seconds,"token":""}),
        )
        .unwrap();
        assert_eq!(
            Settings::decode(&raw).unwrap().public()["pollSeconds"],
            seconds.max(5)
        );
    }
    for seconds in [0, 1, 2, 301] {
        let raw = serde_json::to_vec(
            &json!({"version":1,"enabled":false,"modem":"m1","pollSeconds":seconds,"token":""}),
        )
        .unwrap();
        assert!(Settings::decode(&raw).is_err());
    }
}
#[test]
fn forwarding_budget_is_not_silently_blocked_by_the_manual_thirty_second_cache() {
    let (device, _, _, hub, _) = setup();
    hub.refresh("m1").unwrap();
    assert_eq!(
        hub.refresh_forward("m1", true, 3).err().unwrap().code,
        "SMS_REFRESH_COOLDOWN"
    );
    assert_eq!(device.reads.load(Ordering::SeqCst), 1);
    std::thread::sleep(Duration::from_millis(3050));
    hub.refresh_forward("m1", true, 3).unwrap();
    assert_eq!(device.reads.load(Ordering::SeqCst), 2);
    for _ in 0..10 {
        hub.refresh("m1").unwrap();
        hub.refresh_fresh("m1").unwrap();
        hub.refresh_forward("m1", false, 3).unwrap();
    }
    assert_eq!(
        device.reads.load(Ordering::SeqCst),
        2,
        "Both clients must reuse the same fresh snapshot"
    );
    assert!(hub.refresh_forward("m1", true, 2).is_err());
}
struct Arriving {
    reads: AtomicUsize,
    new: AtomicBool,
}
impl SmsDevice for Arriving {
    fn notice_supported(&self) -> bool {
        true
    }
    fn notice_generation(&self, _: &str) -> u64 {
        u64::from(self.new.load(Ordering::SeqCst))
    }
    fn revision(&self, _: &str) -> Result<String> {
        Ok("one-instance".into())
    }
    fn read(&self, id: &str) -> Result<Vec<Record>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if !self.new.load(Ordering::SeqCst) {
            return Ok(vec![]);
        }
        let pdu = encode_submit("10086", "new fixture")?;
        let mut record = decode(&pdu.hex, pdu.tpdu_bytes, id, "ME", 1, 3)?;
        record.message.direction = wlmb_server::sms::Direction::Incoming;
        Ok(vec![record])
    }
    fn send(&self, _: &str, _: &str, _: &str) -> Result<Value> {
        panic!("No actual SMS send in forwarding tests")
    }
}
#[test]
fn receive_event_triggers_one_shared_scan_and_forwarding_without_periodic_fast_at() {
    let (_, store, build, _, _) = setup();
    let device = Arc::new(Arriving {
        reads: AtomicUsize::new(0),
        new: AtomicBool::new(false),
    });
    let hub = SmsHub::new(device.clone());
    let runtime = ForwardRuntime::new(hub.clone(), store, build.clone()).unwrap();
    let mut settings = enabled();
    settings["pollSeconds"] = json!(300);
    runtime.update(&settings).unwrap();
    wait_ready(&runtime);
    device.new.store(true, Ordering::SeqCst);
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(6) && build.calls.load(Ordering::SeqCst) == 0 {
        if let Err(error) = hub.refresh("m1") {
            assert_eq!(error.code, "SMS_BUSY");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        build.calls.load(Ordering::SeqCst),
        1,
        "The worker must reach PushClient, not only validate settings"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(1800),
        "No overlapping tight-loop scans"
    );
    assert_eq!(device.reads.load(Ordering::SeqCst), 2);
    runtime.shutdown();
    let reads = device.reads.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(device.reads.load(Ordering::SeqCst), reads);
}
#[test]
fn busy_channel_does_not_turn_notifications_into_a_fast_retry_loop() {
    let (device, _, _, _, runtime) = setup();
    device.fail.store(true, Ordering::SeqCst);
    let mut settings = enabled();
    settings["pollSeconds"] = json!(3);
    runtime.update(&settings).unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2)
        && runtime.public().unwrap()["runtime"]["receiveState"] != "waiting"
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(runtime.public().unwrap()["runtime"]["retrySeconds"], 5);
    assert_eq!(device.reads.load(Ordering::SeqCst), 1);
    runtime.shutdown();
}
#[test]
fn receive_only_populates_cache_without_tls_and_both_off_ends_all_work() {
    let (device, store, build, hub, _) = setup();
    let runtime = ForwardRuntime::new(hub.clone(), store, build.clone()).unwrap();
    runtime.update(&json!({"enabled":false,"receiveEnabled":true,"modem":"m1","pollSeconds":300,"token":""})).unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2)
        && runtime.public().unwrap()["runtime"]["receiveState"] != "running"
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        runtime.public().unwrap()["runtime"]["receiveState"],
        "running"
    );
    assert_eq!(device.reads.load(Ordering::SeqCst), 1);
    assert_eq!(hub.cached("m1").unwrap()["loaded"], true);
    assert_eq!(build.creates.load(Ordering::SeqCst), 0);
    runtime.update(&json!({"enabled":false,"receiveEnabled":false,"modem":"m1","pollSeconds":300,"token":""})).unwrap();
    assert_eq!(runtime.public().unwrap()["runtime"]["running"], false);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(device.reads.load(Ordering::SeqCst), 1);
}
#[test]
fn no_event_backend_sleeps_to_deadline_and_releases_without_waiting_for_it() {
    struct Quiet {
        reads: AtomicUsize,
        polls: AtomicUsize,
    }
    impl SmsDevice for Quiet {
        fn poll_notice(&self, _: &str) -> Result<()> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn revision(&self, _: &str) -> Result<String> {
            Ok("quiet-instance".into())
        }
        fn read(&self, _: &str) -> Result<Vec<Record>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        }
        fn send(&self, _: &str, _: &str, _: &str) -> Result<Value> {
            panic!("No send in idle test")
        }
    }
    let (_, store, build, _, _) = setup();
    let device = Arc::new(Quiet {
        reads: AtomicUsize::new(0),
        polls: AtomicUsize::new(0),
    });
    let runtime = ForwardRuntime::new(SmsHub::new(device.clone()), store, build.clone()).unwrap();
    runtime
        .update(
            &json!({"enabled":false,"receiveEnabled":true,"modem":"m1","pollSeconds":5,"token":""}),
        )
        .unwrap();
    wait_ready(&runtime);
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(device.polls.load(Ordering::SeqCst), 1);
    assert_eq!(device.reads.load(Ordering::SeqCst), 1);
    let until = Instant::now() + Duration::from_secs(6);
    while device.reads.load(Ordering::SeqCst) < 2 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        device.reads.load(Ordering::SeqCst),
        2,
        "Deadline still performs fallback collection"
    );
    let started = Instant::now();
    runtime.shutdown();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(build.creates.load(Ordering::SeqCst), 0);
}
#[test]
fn transient_event_busy_retries_without_a_minute_wait_or_an_at_spin_loop() {
    struct Recovering {
        reads: AtomicUsize,
        notice: AtomicUsize,
        fail_next: AtomicBool,
    }
    impl SmsDevice for Recovering {
        fn notice_supported(&self) -> bool {
            true
        }
        fn notice_generation(&self, _: &str) -> u64 {
            self.notice.load(Ordering::SeqCst) as u64
        }
        fn revision(&self, _: &str) -> Result<String> {
            Ok("recovering-instance".into())
        }
        fn read(&self, _: &str) -> Result<Vec<Record>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.fail_next.swap(false, Ordering::SeqCst) {
                Err(PanelError::new("AT_BUSY", 409))
            } else {
                Ok(vec![])
            }
        }
        fn send(&self, _: &str, _: &str, _: &str) -> Result<Value> {
            panic!("No send in retry test")
        }
    }
    let (_, store, build, _, _) = setup();
    let device = Arc::new(Recovering {
        reads: AtomicUsize::new(0),
        notice: AtomicUsize::new(0),
        fail_next: AtomicBool::new(false),
    });
    let runtime = ForwardRuntime::new(SmsHub::new(device.clone()), store, build.clone()).unwrap();
    runtime.update(&json!({"enabled":false,"receiveEnabled":true,"modem":"m1","pollSeconds":300,"token":""})).unwrap();
    wait_ready(&runtime);
    device.fail_next.store(true, Ordering::SeqCst);
    device.notice.store(1, Ordering::SeqCst);
    let until = Instant::now() + Duration::from_secs(5);
    while runtime.public().unwrap()["runtime"]["receiveState"] != "waiting"
        && Instant::now() < until
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(runtime.public().unwrap()["runtime"]["retrySeconds"], 5);
    assert_eq!(device.reads.load(Ordering::SeqCst), 2);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        device.reads.load(Ordering::SeqCst),
        2,
        "Busy state must not spin or replay writes"
    );
    let until = Instant::now() + Duration::from_secs(7);
    while device.reads.load(Ordering::SeqCst) < 3 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(device.reads.load(Ordering::SeqCst), 3);
    assert_eq!(build.creates.load(Ordering::SeqCst), 0);
    runtime.shutdown();
}
#[test]
fn partial_messages_expose_explicit_completion_metadata() {
    struct Partial;
    impl SmsDevice for Partial {
        fn revision(&self, _: &str) -> Result<String> {
            Ok("fixture".into())
        }
        fn read(&self, id: &str) -> Result<Vec<Record>> {
            let pdu = encode_submit("10086", "part")?;
            let mut record = decode(&pdu.hex, pdu.tpdu_bytes, id, "ME", 1, 3)?;
            record.concatenation = Some(wlmb_server::sms_pdu::Concatenation {
                reference: 1,
                total: 3,
                part: 2,
            });
            Ok(vec![record])
        }
        fn send(&self, _: &str, _: &str, _: &str) -> Result<Value> {
            Err(PanelError::new("SMS_UNSUPPORTED", 501))
        }
    }
    let hub = SmsHub::new(Arc::new(Partial));
    let records = hub.refresh("m1").unwrap();
    let data = hub.cached("m1").unwrap();
    assert_eq!(data["messages"][0]["complete"], false);
    assert_eq!(data["messages"][0]["pendingPart"], 2);
    assert_eq!(data["messages"][0]["expectedParts"], 3);
    assert!(wlmb_server::sms_runtime::forwardable(&records).is_empty());
}
#[test]
fn read_state_changes_do_not_query_modem_and_stale_ids_are_rejected() {
    let (d, _, _, hub, _) = setup();
    hub.refresh("m1").unwrap();
    let id = hub.cached("m1").unwrap()["messages"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(hub.cached("m1").unwrap()["messages"][0]["unread"], true);
    hub.mark_read("m1", &id, false).unwrap();
    assert_eq!(hub.cached("m1").unwrap()["messages"][0]["unread"], false);
    hub.mark_read("m1", &id, true).unwrap();
    assert_eq!(hub.cached("m1").unwrap()["messages"][0]["unread"], true);
    assert_eq!(d.reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        hub.mark_read("m1", &"a".repeat(64), false)
            .err()
            .unwrap()
            .code,
        "SMS_CHANGED"
    );
    d.revision.store(2, Ordering::SeqCst);
    assert_eq!(
        hub.mark_read("m1", &id, false).err().unwrap().code,
        "SMS_CHANGED"
    );
}
fn wait_ready(r: &ForwardRuntime) {
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        if r.public().unwrap()["runtime"]["state"] == "running" {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("worker failed to establish baseline");
}
#[test]
fn disabled_has_no_worker_queries_or_tls_factory_and_only_public_presence() {
    let (d, _, build, hub, r) = setup();
    r.start_saved().unwrap();
    let mut v = enabled();
    v["enabled"] = json!(false);
    r.update(&v).unwrap();
    assert_eq!(build.creates.load(Ordering::SeqCst), 0);
    assert_eq!(d.reads.load(Ordering::SeqCst), 0);
    let public = r.public().unwrap().to_string();
    assert!(!public.contains("SecretPush"));
    assert_eq!(r.public().unwrap()["tokenSet"], true);
    assert_eq!(hub.cached("m1").unwrap()["loaded"], false);
}
#[test]
fn enabling_baselines_existing_sms_and_disabling_joins_and_drops_all_resources() {
    let (d, _, build, _, r) = setup();
    r.update(&enabled()).unwrap();
    wait_ready(&r);
    let created = build.creates.load(Ordering::SeqCst);
    r.update(&enabled()).unwrap();
    assert_eq!(
        build.creates.load(Ordering::SeqCst),
        created,
        "Same saved configuration must not restart or rebaseline the worker"
    );
    assert_eq!(d.reads.load(Ordering::SeqCst), 1);
    assert_eq!(build.calls.load(Ordering::SeqCst), 0);
    let journal = String::from_utf8(build.raw.lock().unwrap().clone()).unwrap();
    assert!(!journal.contains("secret") && !journal.contains("10086"));
    let mut v = enabled();
    v["enabled"] = json!(false);
    v["token"] = json!("");
    r.update(&v).unwrap();
    assert_eq!(r.public().unwrap()["runtime"]["running"], false);
    assert_eq!(
        build.creates.load(Ordering::SeqCst),
        build.drops.load(Ordering::SeqCst)
    );
    let count = d.reads.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(d.reads.load(Ordering::SeqCst), count);
    r.update(&json!({"enabled":true,"modem":"m1","pollSeconds":120,"token":""}))
        .unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(1)
        && r.public().unwrap()["runtime"]["receiveState"] != "waiting"
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(r.public().unwrap()["runtime"]["state"], "waiting");
    assert_eq!(build.calls.load(Ordering::SeqCst), 0);
    r.shutdown();
}
#[test]
fn invalid_settings_and_disk_save_failure_do_not_start_polling() {
    let (d, store, build, _, r) = setup();
    let mut v = enabled();
    v["pollSeconds"] = json!(1);
    assert!(r.update(&v).is_err());
    assert_eq!(build.creates.load(Ordering::SeqCst), 0);
    store.fail.store(true, Ordering::SeqCst);
    assert!(r.update(&enabled()).is_err());
    assert_eq!(r.public().unwrap()["enabled"], false);
    assert_eq!(d.reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        build.creates.load(Ordering::SeqCst),
        build.drops.load(Ordering::SeqCst)
    );
    let mut v = enabled();
    v["clearToken"] = json!(true);
    assert!(r.update(&v).is_err());
}
#[test]
fn ui_scans_share_bounded_cache_and_failed_reads_have_cooldown() {
    let (d, _, _, hub, _) = setup();
    hub.refresh("m1").unwrap();
    hub.refresh("m1").unwrap();
    assert_eq!(d.reads.load(Ordering::SeqCst), 1);
    assert_eq!(hub.cached("m1").unwrap()["messages"][0]["phone"], "10086");
    d.fail.store(true, Ordering::SeqCst);
    assert_eq!(hub.refresh("m2").err().unwrap().code, "AT_BUSY");
    assert_eq!(
        hub.refresh("m2").err().unwrap().code,
        "SMS_REFRESH_COOLDOWN"
    );
    assert_eq!(d.reads.load(Ordering::SeqCst), 2);
    assert_eq!(hub.cached("m2").unwrap()["loaded"], false);
    d.revision.store(2, Ordering::SeqCst);
    assert_eq!(
        hub.cached("m1").unwrap()["loaded"],
        false,
        "A replacement modem must not receive old cached SMS"
    );
}
