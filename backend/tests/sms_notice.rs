#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::json;
use std::time::{Duration, Instant};
use wlmb_server::sms_notice::{Notices, fast_scan_allowed, has_cmti};
#[test]
fn index_hints_are_deduplicated_bounded_and_signal_overflow() {
    let mut hints = wlmb_server::sms_notice::Hints::default();
    for i in 0..40 {
        hints.observe(&format!("+CMTI: \"ME\",{i}"));
    }
    assert_eq!(hints.indexes.len(), 32);
    assert!(hints.overflow);
    hints.observe("+CMTI: \"ME\",1\n+CMTI: \"ME\",65536");
    assert_eq!(hints.indexes.len(), 32);
}
#[test]
fn shared_events_are_idle_instance_scoped_deduplicated_and_gap_resynced() {
    use wlmb_server::sms_notice::SharedCursor;
    let row = json!({"id":"m1","port":"/dev/ttyUSB1","model":"fixture","instance":1});
    let event = json!({"port":"/dev/ttyUSB1","correlation":"idle","restart_epoch":1,"sequence":1,"drop_count":0,"raw_line":"+CMTI: \"ME\",1"});
    let mut cursor = SharedCursor::default();
    assert!(cursor.observe(&row, &event));
    assert!(!cursor.observe(&row, &event));
    let mut bad = event.clone();
    bad["sequence"] = json!(2);
    bad["correlation"] = json!("response");
    assert!(!cursor.observe(&row, &bad));
    bad["correlation"] = json!("idle");
    bad["port"] = json!("/dev/ttyUSB2");
    assert!(!cursor.observe(&row, &bad));
    bad["port"] = event["port"].clone();
    bad["raw_line"] = json!("body +CMTI: \"ME\",1");
    assert!(!cursor.observe(&row, &bad));
    bad["sequence"] = json!(3);
    bad["drop_count"] = json!(1);
    assert!(cursor.observe(&row, &bad));
    assert!(!cursor.observe(&row, &bad));
    bad["restart_epoch"] = json!(2);
    bad["sequence"] = json!(1);
    assert!(cursor.observe(&row, &bad));
    bad["sequence"] = json!(2);
    bad["raw_line"] = json!("+CMTI: \"ME\",65536");
    assert!(!cursor.observe(&row, &bad));
}
#[test]
fn notice_is_a_standard_validated_storage_index_not_arbitrary_text() {
    assert!(has_cmti("+CMTI: \"SM\",1\r\nOK\r\n"));
    assert!(has_cmti("+CMTI: \"ME\",0\r\n"));
    for raw in [
        "message +CMTI: \"SM\",1",
        "+CMTI: SM,1",
        "+CMTI: \"SM\",65536",
        "+CMTI: \"SM\",1,2",
        "+CMTI: \"SM\",-1",
        "+CMTI: \"SMS\",1",
    ] {
        assert!(!has_cmti(raw), "{raw}");
    }
}
#[test]
fn notices_are_bounded_instance_scoped_and_never_interpret_sms_content() {
    let notices = Notices::default();
    let row = json!({"id":"m1","port":"/dev/ttyUSB1","model":"fixture","instance":1});
    notices.observe(&row, "AT+CMGL=\"ALL\"", "+CMTI: \"SM\",1");
    assert_eq!(notices.generation(&row), 0);
    notices.observe(&row, "AT^MONSC", "+CMTI: \"SM\",1");
    assert_eq!(notices.generation(&row), 1);
    notices.observe(&row, "AT+CMGF?", "+CMGF:0\n+CMTI: \"SM\",2\nOK");
    assert_eq!(notices.generation(&row), 2);
    let mut changed = row.clone();
    changed["instance"] = json!(2);
    assert_eq!(notices.generation(&changed), 0);
    for n in 0..50 {
        let row = json!({"id":format!("m{n}"),"port":"/dev/ttyUSB1"});
        notices.observe(&row, "AT+CSQ", "+CMTI: \"ME\",2");
    }
    assert_eq!(notices.generation_id("m49"), 0);
}
#[test]
fn fast_receive_budget_coalesces_bursts_and_ignores_clock_reversal() {
    let start = Instant::now();
    assert!(!fast_scan_allowed(
        1,
        0,
        start,
        start + Duration::from_millis(1999)
    ));
    assert!(fast_scan_allowed(
        1,
        0,
        start,
        start + Duration::from_secs(2)
    ));
    assert!(!fast_scan_allowed(
        1,
        1,
        start,
        start + Duration::from_secs(30)
    ));
    assert!(!fast_scan_allowed(
        0,
        1,
        start,
        start + Duration::from_secs(30)
    ));
    assert!(!fast_scan_allowed(
        1,
        0,
        start + Duration::from_secs(3),
        start
    ));
}
#[test]
fn optional_payload_echo_never_hides_unrelated_or_duplicate_send_results() {
    use wlmb_server::sms_session::send_reference_with_pdu_echo;
    assert_eq!(
        send_reference_with_pdu_echo("0011AA\u{1a}\n+CMGS: 7\nOK", "0011AA").unwrap(),
        7
    );
    assert_eq!(
        send_reference_with_pdu_echo("+CMGS: 7\nOK", "0011AA").unwrap(),
        7
    );
    assert!(send_reference_with_pdu_echo("0011AB\n+CMGS: 7\nOK", "0011AA").is_err());
    assert!(send_reference_with_pdu_echo("0011AA\n+CMGS: 7\n+CMGS: 8\nOK", "0011AA").is_err());
}
