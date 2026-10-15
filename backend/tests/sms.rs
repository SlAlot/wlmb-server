#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{cell::Cell, time::Duration};
use wlmb_server::{
    PanelError, Result,
    sms::{self, Direction, Message, PollPlan, ReadMode},
    sms_forward::{Forwarder, Journal, MAX_LEDGER, PushClient, PushOutcome, PushPlus},
};

fn message(index: u32) -> Message {
    Message {
        modem: "m1".into(),
        storage: "ME".into(),
        index,
        direction: Direction::Incoming,
        contact: "10086".into(),
        timestamp: "2026-10-10 09:00:00".into(),
        content: format!("synthetic-{index}"),
    }
}
#[derive(Default)]
struct Store {
    raw: Vec<u8>,
    calls: usize,
    fail_at: usize,
}
impl Journal for Store {
    fn save(&mut self, raw: &[u8]) -> Result<()> {
        self.calls += 1;
        if self.calls == self.fail_at {
            return Err(PanelError::new("TEST_DISK_FULL", 503));
        }
        self.raw = raw.into();
        Ok(())
    }
}
struct Push {
    calls: Cell<usize>,
    outcome: PushOutcome,
}
impl PushClient for Push {
    fn send(&self, _: &Message) -> PushOutcome {
        self.calls.set(self.calls.get() + 1);
        self.outcome.clone()
    }
}
fn ready() -> Forwarder {
    let mut state = Forwarder::default();
    state.set_enabled(true);
    assert_eq!(state.observe("m1", &[message(0)]).unwrap(), 0);
    assert_eq!(state.observe("m1", &[message(0), message(1)]).unwrap(), 1);
    state
}
struct BatchPush {
    batches: std::cell::RefCell<Vec<Vec<u32>>>,
    outcome: PushOutcome,
}
impl PushClient for BatchPush {
    fn send(&self, _: &Message) -> PushOutcome {
        panic!("批量测试不走单条发送");
    }
    fn can_batch(&self, messages: &[Message]) -> bool {
        PushPlus::new("fixture-token-12345678".into())
            .unwrap()
            .body_batch(messages)
            .is_ok()
    }
    fn send_batch(&self, messages: &[Message]) -> PushOutcome {
        self.batches
            .borrow_mut()
            .push(messages.iter().map(|m| m.index).collect());
        self.outcome.clone()
    }
}
#[test]
fn arriving_messages_are_batched_without_extra_wait_and_each_fingerprint_is_reserved() {
    let mut state = ready();
    let all: Vec<_> = (1..=12).map(message).collect();
    state.observe("m1", &all).unwrap();
    let push = BatchPush {
        batches: Default::default(),
        outcome: PushOutcome::Accepted("ref".into()),
    };
    let mut store = Store::default();
    assert!(state.dispatch(Duration::ZERO, &mut store, &push).unwrap());
    assert_eq!(push.batches.borrow()[0], (1..=8).collect::<Vec<_>>());
    assert_eq!(state.status()["accepted"], 8);
    assert_eq!(state.status()["pending"], 4);
    assert!(
        !state
            .dispatch(Duration::from_secs(12), &mut store, &push)
            .unwrap()
    );
    assert!(
        state
            .dispatch(Duration::from_secs(14), &mut store, &push)
            .unwrap()
    );
    assert_eq!(push.batches.borrow()[1], [9, 10, 11, 12]);
    assert_eq!(state.status()["accepted"], 12);
    let mut restored = Forwarder::restore(&store.raw).unwrap();
    restored.set_enabled(true);
    assert_eq!(restored.observe("m1", &all).unwrap(), 0);
}
#[test]
fn batch_disk_failure_never_sends_and_unknown_or_crashed_batch_never_replays() {
    for (fail_at, outcome) in [
        (1, PushOutcome::Unknown),
        (2, PushOutcome::Accepted("ref".into())),
        (0, PushOutcome::Unknown),
    ] {
        let mut state = ready();
        let all: Vec<_> = (1..=4).map(message).collect();
        state.observe("m1", &all).unwrap();
        let push = BatchPush {
            batches: Default::default(),
            outcome,
        };
        let mut store = Store {
            fail_at,
            ..Default::default()
        };
        let result = state.dispatch(Duration::ZERO, &mut store, &push);
        if fail_at == 1 {
            assert!(result.is_err());
            assert!(push.batches.borrow().is_empty());
            assert_eq!(state.status()["pending"], 4);
            assert!(
                state
                    .dispatch(Duration::from_secs(60), &mut store, &push)
                    .unwrap()
            );
        } else {
            assert_eq!(push.batches.borrow().len(), 1);
            let mut restored = Forwarder::restore(&store.raw).unwrap();
            restored.set_enabled(true);
            assert_eq!(restored.observe("m1", &all).unwrap(), 0);
            assert_eq!(restored.status()["unknown"], 4);
        }
    }
}
#[test]
fn push_batches_bound_escaped_payload_and_keep_each_sender_and_timestamp() {
    let client = PushPlus::new("fixture-token-12345678".into()).unwrap();
    let mut a = message(1);
    a.content = "<b>验证码123456</b>".into();
    let mut b = message(2);
    b.contact = "10010".into();
    b.timestamp = "different-time".into();
    let body: Value = serde_json::from_slice(&client.body_batch(&[a, b]).unwrap()).unwrap();
    assert_eq!(body["title"], "网络面板 · 2条新短信");
    let content = body["content"].as_str().unwrap();
    assert!(
        content.contains("&lt;b&gt;")
            && content.contains("10010")
            && content.contains("different-time")
            && content.contains("<hr>")
    );
    assert!(client.body_batch(&[]).is_err());
    assert!(
        client
            .body_batch(&(0..9).map(message).collect::<Vec<_>>())
            .is_err()
    );
    let mut large = message(3);
    large.content = "&".repeat(4096);
    assert!(client.body_batch(&[large.clone()]).is_ok());
    assert!(client.body_batch(&[large.clone(), large]).is_err());
    let mut other = message(4);
    other.modem = "m2".into();
    assert!(client.body_batch(&[message(1), other]).is_err());
}
#[test]
fn large_escaped_messages_split_into_safe_requests_instead_of_rejecting_the_batch() {
    let mut state = Forwarder::default();
    state.set_enabled(true);
    state.observe("m1", &[]).unwrap();
    let mut a = message(1);
    a.content = "&".repeat(4096);
    let mut b = a.clone();
    b.index = 2;
    state.observe("m1", &[a, b]).unwrap();
    let push = BatchPush {
        batches: Default::default(),
        outcome: PushOutcome::Accepted("ref".into()),
    };
    let mut store = Store::default();
    assert!(state.dispatch(Duration::ZERO, &mut store, &push).unwrap());
    assert_eq!(push.batches.borrow()[0], [1]);
    assert_eq!(state.status()["pending"], 1);
    assert!(
        state
            .dispatch(Duration::from_secs(14), &mut store, &push)
            .unwrap()
    );
    assert_eq!(push.batches.borrow()[1], [2]);
    assert_eq!(state.status()["accepted"], 2);
    assert_eq!(state.status()["rejected"], 0);
}
#[test]
fn firmware_schema_directions_and_index_reuse() {
    let raw = serde_json::to_vec(&json!({"smslist":[{"contact":"10086","list":[
        {"role":0,"index":1,"timestamp":1700000000u64,"content":"收到"},
        {"role":1,"index":2,"timestamp":"device-time","content":"发出"},
        {"role":4,"index":3,"content":"unknown"}]}]}))
    .unwrap();
    let rows = sms::parse_smsd(&raw, "m1", "ME").unwrap();
    assert!(rows[0].direction == Direction::Incoming);
    assert!(rows[1].direction == Direction::Outgoing);
    assert!(rows[2].direction == Direction::Unknown);
    let first = rows[0].fingerprint().unwrap();
    let mut changed = rows[0].clone();
    changed.content = "new content".into();
    assert_ne!(first, changed.fingerprint().unwrap());
    changed = rows[0].clone();
    changed.modem = "replacement".into();
    assert_ne!(first, changed.fingerprint().unwrap());
    let duplicate=br#"{"smslist":[{"contact":"10086","list":[{"index":1,"content":"x"},{"index":1,"content":"y"}]}]}"#;
    assert!(sms::parse_smsd(duplicate, "m1", "ME").is_err());
    assert!(sms::parse_smsd(br#"{"smslist":null}"#, "m1", "ME").is_err());
    assert!(sms::parse_smsd(&vec![b' '; 262145], "m1", "ME").is_err());
}
#[test]
fn cold_start_baseline_never_forwards_existing_or_unknown() {
    let mut state = Forwarder::default();
    state.set_enabled(true);
    assert_eq!(state.observe("m1", &[message(0), message(1)]).unwrap(), 0);
    let mut unknown = message(2);
    unknown.direction = Direction::Unknown;
    assert_eq!(
        state
            .observe("m1", &[message(0), message(1), unknown])
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .observe("m1", &[message(0), message(1), message(3)])
            .unwrap(),
        1
    );
    assert_eq!(state.observe("m1", &[message(3)]).unwrap(), 0);
    let raw = state.checkpoint().unwrap();
    let text = String::from_utf8(raw.clone()).unwrap();
    assert!(!text.contains("10086") && !text.contains("synthetic") && !text.contains("2026-10"));
    let mut restored = Forwarder::restore(&raw).unwrap();
    restored.set_enabled(true);
    assert_eq!(restored.observe("m1", &[message(3)]).unwrap(), 1);
    assert_eq!(restored.observe("m2", &[]).unwrap(), 0);
}
#[test]
fn journal_failure_blocks_network_and_ambiguous_result_never_replays() {
    let mut state = ready();
    let push = Push {
        calls: Cell::new(0),
        outcome: PushOutcome::Unknown,
    };
    let mut store = Store {
        fail_at: 1,
        ..Store::default()
    };
    assert!(state.dispatch(Duration::ZERO, &mut store, &push).is_err());
    assert_eq!(push.calls.get(), 0);
    assert!(
        !state
            .dispatch(Duration::from_secs(59), &mut store, &push)
            .unwrap()
    );
    assert!(
        state
            .dispatch(Duration::from_secs(60), &mut store, &push)
            .unwrap()
    );
    assert_eq!(push.calls.get(), 1);
    assert_eq!(state.status()["unknown"], 1);
    let mut restored = Forwarder::restore(&store.raw).unwrap();
    restored.set_enabled(true);
    assert_eq!(
        restored.observe("m1", &[message(0), message(1)]).unwrap(),
        0
    );
    assert!(
        !restored
            .dispatch(Duration::from_secs(600), &mut store, &push)
            .unwrap()
    );
    assert_eq!(push.calls.get(), 1);
}
#[test]
fn crash_after_send_has_durable_reservation() {
    let mut state = ready();
    let push = Push {
        calls: Cell::new(0),
        outcome: PushOutcome::Accepted("reference123".into()),
    };
    let mut store = Store {
        fail_at: 2,
        ..Store::default()
    };
    assert!(state.dispatch(Duration::ZERO, &mut store, &push).is_err());
    assert_eq!(push.calls.get(), 1);
    assert!(
        String::from_utf8(store.raw.clone())
            .unwrap()
            .contains("inFlight")
    );
    let mut restored = Forwarder::restore(&store.raw).unwrap();
    restored.set_enabled(true);
    assert_eq!(restored.status()["unknown"], 1);
    assert_eq!(restored.observe("m1", &[message(1)]).unwrap(), 0);
    assert!(Forwarder::restore(b"corrupt").is_err());
}
#[test]
fn forward_throttle_disable_and_capacity_never_clear_history() {
    let mut state = ready();
    state.observe("m1", &[message(1), message(2)]).unwrap();
    let push = Push {
        calls: Cell::new(0),
        outcome: PushOutcome::Accepted("ref".into()),
    };
    let mut store = Store::default();
    assert!(state.dispatch(Duration::ZERO, &mut store, &push).unwrap());
    assert!(
        !state
            .dispatch(Duration::from_secs(9), &mut store, &push)
            .unwrap()
    );
    state.set_enabled(false);
    state.set_enabled(true);
    assert_eq!(state.observe("m1", &[message(2)]).unwrap(), 0);
    assert!(
        !state
            .dispatch(Duration::from_secs(10), &mut store, &push)
            .unwrap()
    );
    let mut fresh = Forwarder::default();
    fresh.set_enabled(true);
    fresh.observe("m1", &[]).unwrap();
    let oversized: Vec<_> = (0..33).map(message).collect();
    assert!(fresh.observe("m1", &oversized).is_err());
    assert_eq!(fresh.status()["pending"], 0);
    let records: serde_json::Map<String, Value> = (0..MAX_LEDGER)
        .map(|n| (format!("{n:064x}"), json!("baseline")))
        .collect();
    let checkpoint =
        serde_json::to_vec(&json!({"version":1,"baselines":[],"records":records})).unwrap();
    let mut full = Forwarder::restore(&checkpoint).unwrap();
    assert!(full.observe("m1", &[message(1)]).is_err());
    assert_eq!(full.status()["historyFull"], true);
}
#[test]
fn fast_modem_poll_does_not_exceed_the_ordinary_pushplus_request_rate() {
    let mut state = ready();
    state
        .observe("m1", &(1..=7).map(message).collect::<Vec<_>>())
        .unwrap();
    let push = Push {
        calls: Cell::new(0),
        outcome: PushOutcome::Accepted("ref".into()),
    };
    let mut store = Store::default();
    assert!(state.dispatch(Duration::ZERO, &mut store, &push).unwrap());
    for seconds in [3, 5, 10, 12] {
        assert!(
            !state
                .dispatch(Duration::from_secs(seconds), &mut store, &push)
                .unwrap()
        );
    }
    for millis in [13100, 26200, 39300, 52400] {
        assert!(
            state
                .dispatch(Duration::from_millis(millis), &mut store, &push)
                .unwrap()
        );
    }
    assert!(
        !state
            .dispatch(Duration::from_secs(59), &mut store, &push)
            .unwrap()
    );
    assert_eq!(push.calls.get(), 5);
}
#[test]
fn notification_storm_busy_backoff_and_disable_do_not_overlap() {
    let mut plan = PollPlan::new(ReadMode::SharedAtPolling);
    assert!(plan.begin(Duration::from_secs(1000), false).is_none());
    plan.enable(true, Duration::ZERO);
    assert!(plan.begin(Duration::from_secs(29), false).is_none());
    assert!(plan.begin(Duration::from_secs(30), true).is_none());
    let ticket = plan.begin(Duration::from_secs(45), false).unwrap();
    for _ in 0..1000 {
        plan.notify(Duration::from_secs(46));
    }
    assert!(plan.begin(Duration::from_secs(999), false).is_none());
    assert!(plan.finish(ticket, Duration::from_secs(46), Ok(false)));
    assert_eq!(plan.next_due(), Some(Duration::from_secs(76)));
    let ticket = plan.begin(Duration::from_secs(76), false).unwrap();
    plan.enable(false, Duration::from_secs(77));
    plan.enable(true, Duration::from_secs(78));
    assert!(plan.begin(Duration::from_secs(500), false).is_none());
    assert!(!plan.finish(ticket, Duration::from_secs(80), Ok(true)));
    assert_eq!(plan.next_due(), Some(Duration::from_secs(110)));
    let ticket = plan.begin(Duration::from_secs(110), false).unwrap();
    assert!(plan.finish(
        ticket,
        Duration::from_secs(111),
        Err(PanelError::new("BUSY", 503))
    ));
    assert_eq!(plan.next_due(), Some(Duration::from_secs(171)));
}
#[test]
fn idle_poll_budget_is_minutes_not_signal_refresh() {
    let mut plan = PollPlan::new(ReadMode::SharedAtPolling);
    plan.enable(true, Duration::ZERO);
    let ticket = plan.begin(Duration::from_secs(30), false).unwrap();
    plan.finish(ticket, Duration::from_secs(31), Ok(false));
    assert_eq!(plan.next_due(), Some(Duration::from_secs(151)));
    let mut events = PollPlan::new(ReadMode::VerifiedDaemonEvents);
    events.enable(true, Duration::ZERO);
    let ticket = events.begin(Duration::from_secs(30), false).unwrap();
    events.finish(ticket, Duration::from_secs(31), Ok(false));
    assert_eq!(events.next_due(), Some(Duration::from_secs(331)));
}

#[test]
fn events_do_not_cancel_error_backoff() {
    let mut plan = PollPlan::new(ReadMode::VerifiedDaemonEvents);
    plan.enable(true, Duration::ZERO);
    let ticket = plan.begin(Duration::from_secs(30), false).unwrap();
    plan.notify(Duration::from_secs(31));
    plan.finish(
        ticket,
        Duration::from_secs(32),
        Err(PanelError::new("UNSUPPORTED", 503)),
    );
    for _ in 0..1000 {
        plan.notify(Duration::from_secs(33));
    }
    assert_eq!(plan.next_due(), Some(Duration::from_secs(92)));
    assert!(plan.begin(Duration::from_secs(91), false).is_none());
}

#[test]
fn forwarding_cooldown_starts_after_slow_request_finishes() {
    struct Slow;
    impl PushClient for Slow {
        fn send(&self, _: &Message) -> PushOutcome {
            std::thread::sleep(Duration::from_millis(30));
            PushOutcome::Accepted("ref".into())
        }
    }
    let mut state = ready();
    state.observe("m1", &[message(1), message(2)]).unwrap();
    let mut store = Store::default();
    state.dispatch(Duration::ZERO, &mut store, &Slow).unwrap();
    assert!(
        !state
            .dispatch(Duration::from_millis(13001), &mut store, &Slow)
            .unwrap()
    );
    assert!(
        state
            .dispatch(Duration::from_secs(14), &mut store, &Slow)
            .unwrap()
    );
}

#[cfg(unix)]
#[test]
#[ignore = "Only run as root inside the isolated OpenWrt lab; never on a supply router"]
fn isolated_private_journal_atomic_permissions_and_symlink_rejection() {
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt, symlink},
    };
    use wlmb_server::sms_forward::DiskJournal;
    assert!(std::path::Path::new("/etc/network-panel-isolated-lab").is_file());
    let path = std::env::temp_dir().join(format!("wlmb-sms-test-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    let mut store = DiskJournal::open(&path).unwrap();
    assert!(store.load().unwrap().is_none());
    let mut state = ready();
    let push = Push {
        calls: Cell::new(0),
        outcome: PushOutcome::Unknown,
    };
    state.dispatch(Duration::ZERO, &mut store, &push).unwrap();
    assert_eq!(store.load().unwrap().unwrap().status()["unknown"], 1);
    let file = path.join("sms-forward.json");
    assert_eq!(fs::metadata(&file).unwrap().mode() & 0o777, 0o600);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.load().is_err());
    fs::remove_file(&file).unwrap();
    symlink("/etc/passwd", &file).unwrap();
    assert!(store.load().is_err());
    fs::remove_file(&file).unwrap();
    fs::remove_dir(&path).unwrap();
}
#[test]
fn pushplus_native_payload_and_receipt_are_bounded_private_and_not_delivery() {
    let client = PushPlus::new("synthetic-token-only".into()).unwrap();
    let mut row = message(1);
    row.content = "<script>alert('test')</script> & more".into();
    let value: Value = serde_json::from_slice(&client.body(&row).unwrap()).unwrap();
    assert!(!value["content"].as_str().unwrap().contains("<script>"));
    assert!(
        value["content"]
            .as_str()
            .unwrap()
            .contains("&lt;script&gt;")
    );
    assert_eq!(value["channel"], "wechat");
    assert!(value.get("callbackUrl").is_none());
    assert!(
        PushPlus::response(200, br#"{"code":200,"data":"reference"}"#)
            == PushOutcome::Accepted("reference".into())
    );
    assert!(PushPlus::response(200, br#"{"code":200}"#) == PushOutcome::Unknown);
    assert!(
        PushPlus::response(200, br#"{"code":600,"msg":"secret content"}"#) == PushOutcome::Rejected
    );
    assert!(PushPlus::response(302, b"redirect") == PushOutcome::Unknown);
    assert!(PushPlus::new("bad\ntoken".into()).is_err());
}
