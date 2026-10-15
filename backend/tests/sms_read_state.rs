#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use wlmb_server::{
    PanelError, Result,
    sms::{Direction, Message},
    sms_read_state::{Ledger, ReadStore},
};
struct Memory {
    raw: Mutex<Option<Vec<u8>>>,
    fail: AtomicBool,
}
impl ReadStore for Memory {
    fn load(&self) -> Result<Option<Vec<u8>>> {
        Ok(self.raw.lock().unwrap().clone())
    }
    fn save(&self, raw: &[u8]) -> Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(PanelError::new("SMS_READ_STATE_FAILED", 503));
        }
        *self.raw.lock().unwrap() = Some(raw.into());
        Ok(())
    }
}
fn message() -> Message {
    Message {
        modem: "m1".into(),
        storage: "ME".into(),
        index: 1,
        direction: Direction::Incoming,
        contact: "10086".into(),
        timestamp: "2026-10-11 12:00:00".into(),
        content: "fixture private body".into(),
    }
}
#[test]
fn plugin_read_state_survives_restart_and_slot_reordering_without_private_data() {
    let store = Arc::new(Memory {
        raw: Mutex::new(None),
        fail: AtomicBool::new(false),
    });
    let mut ledger = Ledger::open(store.clone()).unwrap();
    let mut row = message();
    assert!(ledger.unread("hardware1", &row).unwrap());
    ledger.mark("hardware1", &row, false).unwrap();
    row.index = 8;
    let mut restarted = Ledger::open(store.clone()).unwrap();
    assert!(!restarted.unread("hardware1", &row).unwrap());
    assert!(restarted.unread("hardware2", &row).unwrap());
    let raw = String::from_utf8(store.raw.lock().unwrap().clone().unwrap()).unwrap();
    assert!(!raw.contains("10086"));
    assert!(!raw.contains("private body"));
    assert!(!raw.contains("hardware1"));
    restarted.mark("hardware1", &row, true).unwrap();
    assert!(restarted.unread("hardware1", &row).unwrap());
}
#[test]
fn changed_message_and_failed_save_never_claim_success() {
    let store = Arc::new(Memory {
        raw: Mutex::new(None),
        fail: AtomicBool::new(false),
    });
    let mut ledger = Ledger::open(store.clone()).unwrap();
    let row = message();
    store.fail.store(true, Ordering::SeqCst);
    assert!(ledger.mark("hardware1", &row, false).is_err());
    assert!(ledger.unread("hardware1", &row).unwrap());
    store.fail.store(false, Ordering::SeqCst);
    ledger.mark("hardware1", &row, false).unwrap();
    let mut new = row.clone();
    new.content = "new message same slot".into();
    assert!(ledger.unread("hardware1", &new).unwrap());
    ledger.reconcile("hardware1", &[new]).unwrap();
    assert!(ledger.unread("hardware1", &row).unwrap());
    let mut sent = row;
    sent.direction = Direction::Outgoing;
    assert!(!ledger.unread("hardware1", &sent).unwrap());
    assert!(ledger.mark("hardware1", &sent, false).is_err());
}
#[test]
fn malformed_ledger_is_rejected_instead_of_silently_reset() {
    let store = Arc::new(Memory {
        raw: Mutex::new(Some(br#"{"version":1,"read":{"bad":["hash"]}}"#.to_vec())),
        fail: AtomicBool::new(false),
    });
    assert!(Ledger::open(store).is_err());
}
