#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::PanelError;
#[test]
fn both_frontends_receive_explicit_non_retryable_outcomes() {
    for (code, status, category, changed) in [
        ("LOCK_BAND_PROTOCOL_UNSUPPORTED", 422, "pending", false),
        ("AT_UNSUPPORTED", 422, "unsupported", false),
        ("AT_BUSY", 409, "busy", false),
        ("SHARED_QUEUE_CHANGED", 409, "changed", false),
        ("LOCK_CHANGED", 409, "changed", false),
        ("MODEM_CHANGED", 409, "changed", false),
        ("LOCK_RESULT_UNKNOWN", 409, "uncertain", true),
        ("SMS_SEND_UNKNOWN", 503, "uncertain", true),
        ("SMS_DELETE_PARTIAL", 409, "uncertain", true),
        ("WIFI_RELOAD_UNKNOWN", 503, "uncertain", true),
        ("LOCK_CONFIRMATION_REQUIRED", 409, "confirmation", false),
    ] {
        let error = PanelError::new(code, status).details();
        assert_eq!(error["category"], category);
        assert_eq!(error["operationMayHaveApplied"], changed);
        assert_eq!(error["automaticRetry"], false);
        assert_ne!(error["message"], code);
        assert!(error.get("raw").is_none());
    }
}
#[test]
fn legacy_shared_message_no_longer_falsely_requires_independent_mode() {
    let message = PanelError::new("SMS_SHARED_READ_ONLY", 422).message();
    assert!(message.contains("事务保护"));
    assert!(!message.contains("需要独立"));
}
