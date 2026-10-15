#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use wlmb_server::{
    PanelError, Result,
    qmodem_sms::{QmodemSms, Service, record},
    sms_pdu::Record,
    sms_runtime::SmsDevice,
};
fn message(id: u64) -> Value {
    json!({"id":id,"type":"received","sender":"10086","timestamp":1791900000u64,"content":"synthetic 123456","complete":1,"revision":1,"is_read":0})
}
fn schema() -> Value {
    json!({"status":{},"config_get":{"modem_id":3},"sync":{"modem_id":3},"list":{"modem_id":3,"limit":5,"offset":5},"get":{"modem_id":3,"id":0},"delete":{"modem_id":3,"id":0},"send_managed":{"modem_id":3,"recipient":3,"content":3,"request_id":3}})
}
struct Fixture {
    identities: AtomicUsize,
    mode: Mutex<String>,
    rows: Mutex<Vec<Value>>,
    calls: Mutex<Vec<(String, Value)>>,
    events: Mutex<VecDeque<Value>>,
    schema: Mutex<Option<Value>>,
    fail: Mutex<Option<String>>,
    identity: Mutex<Value>,
    api: AtomicUsize,
}
impl Service for Fixture {
    fn identity(&self, _: &str) -> Result<Value> {
        self.identities.fetch_add(1, Ordering::SeqCst);
        Ok(self.identity.lock().unwrap().clone())
    }
    fn describe(&self) -> Result<Value> {
        self.schema
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| PanelError::new("UBUS_OBJECT_NOT_FOUND", 503))
    }
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        if self.fail.lock().unwrap().as_deref() == Some(method) {
            return Err(PanelError::new("SYSTEM_UNAVAILABLE", 503));
        }
        if method != "status" {
            assert_eq!(params["modem_id"], "modem1");
        }
        match method {
            "status" => Ok(json!({"status":"ready","api_version":self.api.load(Ordering::SeqCst)})),
            "config_get" => Ok(
                json!({"status":"success","mode":*self.mode.lock().unwrap(),"auto_delete":1,"forwarding":1}),
            ),
            "sync" => Ok(json!({"status":"success","imported":0})),
            "list" => {
                assert_eq!(params["limit"], 8);
                let offset = params["offset"].as_u64().unwrap() as usize;
                let rows = self.rows.lock().unwrap();
                let window: Vec<_> = if rows.len() > 128 {
                    rows.iter().collect()
                } else {
                    rows.iter().skip(offset).take(8).collect()
                };
                Ok(json!({"mode":*self.mode.lock().unwrap(),"messages":window}))
            }
            "get" => self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r["id"] == params["id"])
                .cloned()
                .ok_or_else(|| PanelError::new("SYSTEM_UNAVAILABLE", 503)),
            "send_managed" => {
                assert_eq!(params["request_id"].as_str().unwrap().len(), 32);
                Ok(json!({"status":"success","parts":1}))
            }
            "delete" => Ok(json!({"status":"success","deleted":1})),
            _ => panic!("unreviewed call: {method}"),
        }
    }
    fn event(&self) -> Result<Option<Value>> {
        Ok(self.events.lock().unwrap().pop_front())
    }
    fn event_connected(&self) -> bool {
        true
    }
}
#[derive(Default)]
struct Fallback(AtomicUsize);
impl SmsDevice for Fallback {
    fn revision(&self, _: &str) -> Result<String> {
        Ok("fallback".into())
    }
    fn read(&self, _: &str) -> Result<Vec<Record>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    fn send(&self, _: &str, _: &str, _: &str) -> Result<Value> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({}))
    }
}
fn setup() -> (QmodemSms, Arc<Fixture>, Arc<Fallback>) {
    let service = Arc::new(Fixture {
        identities: AtomicUsize::new(0),
        mode: Mutex::new("database_poll".into()),
        rows: Mutex::new(vec![message(70000)]),
        calls: Mutex::new(vec![]),
        events: Mutex::new(VecDeque::new()),
        schema: Mutex::new(Some(schema())),
        fail: Mutex::new(None),
        identity: Mutex::new(
            json!({"id":"modem1","port":"/dev/ttyUSB1","smsPort":"/dev/ttyUSB2","model":"fixture-model","shared":true,"instance":"1:2","discoveredBy":"qmodem-config"}),
        ),
        api: AtomicUsize::new(3),
    });
    let fallback = Arc::new(Fallback::default());
    (
        QmodemSms::new(service.clone(), fallback.clone()),
        service,
        fallback,
    )
}
#[test]
fn database_import_is_modem_bound_and_does_not_duplicate_shared_at() {
    let (device, service, fallback) = setup();
    let rows = device.read("modem1").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].message.storage, "QDB1");
    assert_eq!(rows[0].message.index, 4464);
    assert!(device.writable_for("modem1"));
    assert!(!device.writable());
    assert_eq!(device.source_info("modem1")["deleteScope"], "qmodem-record");
    assert_eq!(fallback.0.load(Ordering::SeqCst), 0);
    let calls = service.calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|(m, _)| m == "sync").count(), 1);
    assert!(!calls.iter().any(|(m, _)| {
        ["configure", "mark_read", "delivery_claim", "storage_set"].contains(&m.as_str())
    }));
}
#[test]
fn urc_burst_is_bounded_coalesced_and_does_not_import_or_repeat_identity_for_each_event() {
    let (device, service, fallback) = setup();
    *service.mode.lock().unwrap() = "database_urc".into();
    device.read("modem1").unwrap();
    service.calls.lock().unwrap().clear();
    let before = service.identities.load(Ordering::SeqCst);
    for seq in 1..=40 {
        service.events.lock().unwrap().push_back(json!({"port":"/dev/ttyUSB2","owner":"qmodem.sms","urc_id":"sms-new","restart_epoch":1,"sequence":seq}));
    }
    device.poll_notice("modem1").unwrap();
    assert_eq!(device.notice_generation("modem1"), 32);
    assert_eq!(service.events.lock().unwrap().len(), 8);
    assert_eq!(service.identities.load(Ordering::SeqCst) - before, 1);
    device.poll_notice("modem1").unwrap();
    assert_eq!(device.notice_generation("modem1"), 40);
    assert_eq!(service.identities.load(Ordering::SeqCst) - before, 2);
    assert!(service.calls.lock().unwrap().is_empty());
    assert_eq!(fallback.0.load(Ordering::SeqCst), 0);
}
#[test]
fn repeated_ui_capabilities_reuse_verified_snapshot_but_writes_still_check_identity() {
    let (device, service, _) = setup();
    let row = service.identity.lock().unwrap().clone();
    assert!(device.writable_snapshot(&row));
    let before = service.identities.load(Ordering::SeqCst);
    for _ in 0..1000 {
        assert!(device.writable_snapshot(&row));
    }
    assert_eq!(
        service.identities.load(Ordering::SeqCst),
        before,
        "界面刷新不反复执行模组发现"
    );
    let calls = service.calls.lock().unwrap().len();
    device.send("modem1", "10086", "synthetic").unwrap();
    assert!(service.identities.load(Ordering::SeqCst) > before);
    assert!(service.calls.lock().unwrap().len() > calls);
}
#[test]
fn database_keys_preserve_full_i64_and_do_not_alias_physical_slots() {
    let ids = [1, 65535, 65536, 70000, (1u64 << 53) + 7, i64::MAX as u64];
    let mut fingerprints = std::collections::BTreeSet::new();
    for id in ids {
        let (key, r) = record(&message(id), "modem1").unwrap().unwrap();
        assert_eq!(key, id);
        assert!(r.message.storage.len() <= 16);
        assert!(fingerprints.insert(r.message.fingerprint().unwrap()));
    }
    assert!(record(&message(0), "modem1").is_err());
    assert!(record(&message(u64::MAX), "modem1").is_err());
}
#[test]
fn long_history_uses_bounded_pages_instead_of_overflowing_native_ubus_frames() {
    let (device, service, _) = setup();
    *service.rows.lock().unwrap() = (1..=128)
        .map(|id| {
            let mut row = message(id);
            row["content"] = json!("x".repeat(2048));
            row
        })
        .collect();
    assert!(device.read("modem1").is_err());
    *service.rows.lock().unwrap() = (1..=40).map(message).collect();
    let rows = device.read("modem1").unwrap();
    assert_eq!(rows.len(), 40);
    assert!(
        service
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(m, p)| m == "list" && p["offset"] == 40)
    );
}
#[test]
fn empty_database_is_normal_and_incomplete_records_wait_for_owner_assembly() {
    let (device, service, _) = setup();
    service.rows.lock().unwrap().clear();
    assert!(device.read("modem1").unwrap().is_empty());
    let mut partial = message(1);
    partial["complete"] = json!(0);
    *service.rows.lock().unwrap() = vec![partial];
    assert!(device.read("modem1").unwrap().is_empty());
    service.rows.lock().unwrap()[0]["complete"] = json!(1);
    assert_eq!(device.read("modem1").unwrap().len(), 1);
}
#[test]
fn duplicate_oversized_unknown_direction_and_malformed_snapshots_fail() {
    let (device, service, _) = setup();
    for rows in [
        vec![message(1), message(1)],
        vec![message(1); 129],
        vec![json!({"id":1,"complete":true,"type":"other"})],
    ] {
        *service.rows.lock().unwrap() = rows;
        assert!(device.read("modem1").is_err());
    }
    let mut row = message(1);
    row["content"] = json!("x".repeat(4097));
    assert!(record(&row, "modem1").is_err());
}
#[test]
fn present_but_incompatible_api_or_descriptor_never_falls_back_to_at() {
    let (device, service, fallback) = setup();
    service.api.store(4, Ordering::SeqCst);
    assert!(device.read("modem1").is_err());
    assert_eq!(fallback.0.load(Ordering::SeqCst), 0);
    service.api.store(3, Ordering::SeqCst);
    service.schema.lock().unwrap().as_mut().unwrap()["list"]["modem_id"] = json!(5);
    assert!(device.read("modem1").is_err());
    assert_eq!(fallback.0.load(Ordering::SeqCst), 0);
}
#[test]
fn absent_or_explicit_direct_service_uses_existing_safe_fallback() {
    let (device, service, fallback) = setup();
    *service.schema.lock().unwrap() = None;
    device.read("modem1").unwrap();
    assert_eq!(fallback.0.load(Ordering::SeqCst), 1);
    let (device, service, fallback) = setup();
    *service.mode.lock().unwrap() = "direct".into();
    device.read("modem1").unwrap();
    assert_eq!(fallback.0.load(Ordering::SeqCst), 1);
}
#[test]
fn service_failures_and_mode_changes_after_selection_never_switch_sources() {
    let (device, service, fallback) = setup();
    device.read("modem1").unwrap();
    *service.fail.lock().unwrap() = Some("sync".into());
    assert!(device.read("modem1").is_err());
    *service.fail.lock().unwrap() = None;
    *service.mode.lock().unwrap() = "direct".into();
    assert_eq!(
        device.read("modem1").err().unwrap().code,
        "SMS_STORAGE_CHANGED"
    );
    assert_eq!(fallback.0.load(Ordering::SeqCst), 0);
}
#[test]
fn managed_send_timeout_is_uncertain_single_attempt_and_never_at_replayed() {
    let (device, service, fallback) = setup();
    device.read("modem1").unwrap();
    *service.fail.lock().unwrap() = Some("send_managed".into());
    assert_eq!(
        device
            .send("modem1", "10086", "synthetic")
            .err()
            .unwrap()
            .code,
        "SMS_SEND_UNKNOWN"
    );
    assert_eq!(
        service
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "send_managed")
            .count(),
        1
    );
    assert_eq!(fallback.0.load(Ordering::SeqCst), 0);
}
#[test]
fn deletion_rechecks_fingerprint_and_passes_database_id_not_sim_index() {
    let (device, service, _) = setup();
    let rows = device.read("modem1").unwrap();
    let id = rows[0].message.fingerprint().unwrap();
    assert_eq!(
        device.delete("modem1", &id).unwrap()["deleteScope"],
        "qmodem-record"
    );
    let calls = service.calls.lock().unwrap();
    let request = &calls.iter().find(|(m, _)| m == "delete").unwrap().1;
    assert_eq!(request["id"], 70000);
    assert!(request.get("index").is_none());
    drop(calls);
    device.read("modem1").unwrap();
    service.rows.lock().unwrap()[0]["content"] = json!("changed synthetic");
    assert_eq!(
        device.delete("modem1", &id).err().unwrap().code,
        "SMS_STORAGE_CHANGED"
    );
    assert_eq!(
        service
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "delete")
            .count(),
        1
    );
}
#[test]
fn urc_owner_and_sms_port_are_bound_and_duplicate_events_do_not_scan() {
    let (device, service, _) = setup();
    *service.mode.lock().unwrap() = "database_urc".into();
    device.read("modem1").unwrap();
    let base = json!({"port":"/dev/ttyUSB2","owner":"qmodem.sms","urc_id":"sms-new","restart_epoch":10,"sequence":1});
    for (field, value) in [
        ("port", json!("/dev/ttyUSB1")),
        ("owner", json!("other")),
        ("urc_id", json!("other")),
    ] {
        let mut wrong = base.clone();
        wrong[field] = value;
        service.events.lock().unwrap().push_back(wrong);
        device.poll_notice("modem1").unwrap();
        assert_eq!(device.notice_generation("modem1"), 0);
    }
    service.events.lock().unwrap().push_back(base.clone());
    device.poll_notice("modem1").unwrap();
    assert_eq!(device.notice_generation("modem1"), 1);
    service.events.lock().unwrap().push_back(base);
    device.poll_notice("modem1").unwrap();
    assert_eq!(device.notice_generation("modem1"), 1);
    service.calls.lock().unwrap().clear();
    device.read_event("modem1").unwrap();
    assert!(
        !service
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(m, _)| m == "sync"),
        "URC mode consumes database, not a redundant AT import"
    );
}
#[test]
fn model_agnostic_identity_changes_invalidate_old_database_delete_targets() {
    let (device, service, _) = setup();
    let rows = device.read("modem1").unwrap();
    let fp = rows[0].message.fingerprint().unwrap();
    let before = device.revision("modem1").unwrap();
    service.identity.lock().unwrap()["instance"] = json!("1:3");
    assert_ne!(before, device.revision("modem1").unwrap());
    assert_eq!(
        device.delete("modem1", &fp).err().unwrap().code,
        "SMS_STORAGE_CHANGED"
    );
    assert!(
        !service
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(m, _)| m == "delete")
    );
}
