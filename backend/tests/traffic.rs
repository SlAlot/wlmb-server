#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::json;
use wlmb_server::traffic::{Accounting, Budget, Date};
fn date(s: &str) -> Option<Date> {
    Some(Date::parse(s).unwrap())
}
#[test]
fn real_delta_not_boot_counter_and_reset_or_interface_change_rebaseline() {
    let mut a = Accounting::default();
    a.sample(date("2026-10-10"), Some(("wan", 10000, 20000)))
        .unwrap();
    assert_eq!(a.public()["monthBytes"], 0);
    a.sample(date("2026-10-10"), Some(("wan", 10100, 20200)))
        .unwrap();
    assert_eq!(a.public()["todayBytes"], 300);
    a.sample(date("2026-10-10"), Some(("wan", 1, 2))).unwrap();
    a.sample(date("2026-10-10"), Some(("usb", 1000, 2000)))
        .unwrap();
    assert_eq!(a.public()["monthBytes"], 300);
    a.sample(date("2026-10-10"), None).unwrap();
    assert!(a.public()["monthBytes"].is_null() || !a.observed);
    a.sample(date("2026-10-10"), Some(("usb", 1100, 2100)))
        .unwrap();
    assert_eq!(a.public()["monthBytes"], 300);
}

#[test]
fn recreated_same_name_interface_does_not_import_a_larger_counter() {
    let mut a = Accounting::default();
    a.sample(date("2026-10-11"), Some(("wwan0#10", 100, 100)))
        .unwrap();
    a.sample(date("2026-10-11"), Some(("wwan0#10", 200, 200)))
        .unwrap();
    a.sample(date("2026-10-11"), Some(("wwan0#11", 10000, 10000)))
        .unwrap();
    assert_eq!(a.public()["todayBytes"], 200);
    a.sample(date("2026-10-11"), Some(("wwan0#11", 10100, 10000)))
        .unwrap();
    assert_eq!(a.public()["monthBytes"], 300);
}
#[test]
fn cycle_day_year_and_clock_rollback_do_not_erase_history() {
    let mut a = Accounting::default();
    a.budget.start = 10;
    for (day, n) in [("2026-12-09", 0), ("2026-12-10", 100), ("2027-01-09", 300)] {
        a.sample(date(day), Some(("wan", n, 0))).unwrap();
    }
    assert_eq!(a.public()["monthBytes"], 300);
    a.sample(date("2026-12-01"), Some(("wan", 400, 0))).unwrap();
    assert!(a.public()["monthBytes"].is_null());
    a.sample(date("2027-01-09"), Some(("wan", 500, 0))).unwrap();
    assert_eq!(a.public()["monthBytes"], 400);
    a.sample(date("2027-01-10"), Some(("wan", 550, 0))).unwrap();
    assert_eq!(a.public()["monthBytes"], 50);
}
#[test]
fn checkpoints_restore_history_but_not_counter_or_clock_claims() {
    let mut a = Accounting::default();
    a.sample(date("2026-10-10"), Some(("wan", 0, 0))).unwrap();
    a.sample(date("2026-10-10"), Some(("wan", 500, 0))).unwrap();
    let mut b = Accounting::restore(&a.checkpoint().unwrap()).unwrap();
    assert!(b.public()["monthBytes"].is_null());
    b.sample(date("2026-10-10"), Some(("wan", 9999, 0)))
        .unwrap();
    assert_eq!(b.public()["monthBytes"], 500);
    b.sample(date("2026-10-11"), Some(("wan", 10099, 0)))
        .unwrap();
    assert_eq!(b.public()["todayBytes"], 100);
    assert_eq!(b.public()["monthBytes"], 600);
}
#[test]
fn budget_and_dates_fail_closed_without_panicking_on_unicode() {
    for d in [
        "2026-02-29",
        "2026-13-01",
        "界界界-01-01",
        "2026-00-10",
        "1970-01-01",
    ] {
        assert!(Date::parse(d).is_err());
    }
    assert!(Date::parse("2028-02-29").is_ok());
    let v = json!({"enabled":true,"limitBytes":1000,"startDay":10,"threshold":90,"cutoffEnabled":false});
    let b = Budget::decode(&v).unwrap();
    assert_eq!(b.public(900, true)["warning"], true);
    let mut bad = v;
    bad["startDay"] = json!(31);
    assert!(Budget::decode(&bad).is_err());
}

#[test]
fn legacy_checkpoint_preserves_totals_without_replaying_old_counters() {
    let legacy = json!({"format":1,"day":"2026-10-10","month":"2026-10-01",
        "boot":"old-boot","interface":"eth2","rx":38038286347_u64,
        "tx":6276795929_u64,"todayBytes":30371305271_u64,"monthBytes":37050629326_u64});
    let mut a = Accounting::restore(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(a.public()["todayBytes"].is_null());
    assert!(!a.budget.enabled);
    a.sample(date("2026-10-10"), Some(("eth2", 50000000000, 9000000000)))
        .unwrap();
    assert_eq!(a.public()["todayBytes"], 30371305271_u64);
    assert_eq!(a.public()["monthBytes"], 37050629326_u64);
    a.sample(date("2026-10-10"), Some(("eth2", 50000000100, 9000000200)))
        .unwrap();
    assert_eq!(a.public()["monthBytes"], 37050629626_u64);
    let mut b = Accounting::restore(&a.checkpoint().unwrap()).unwrap();
    b.sample(date("2026-10-11"), Some(("eth2", 99, 99)))
        .unwrap();
    assert_eq!(b.public()["todayBytes"], 0);
    assert_eq!(b.public()["monthBytes"], 37050629626_u64);
}

#[test]
fn malformed_or_ambiguous_legacy_checkpoints_fail_closed() {
    let valid = json!({"format":1,"day":"2026-10-10","month":"2026-10-01",
        "todayBytes":100,"monthBytes":200});
    for (key, value) in [
        ("format", json!(2)),
        ("day", json!("2026-02-29")),
        ("month", json!("2026-09-01")),
        ("month", json!("2026-10-02")),
        ("todayBytes", json!(201)),
        ("monthBytes", json!(-1)),
        ("version", json!(null)),
    ] {
        let mut bad = valid.clone();
        bad[key] = value;
        assert!(Accounting::restore(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
}
