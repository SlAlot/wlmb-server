#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::sms_session::{pdu_mode, send_reference, storage};
#[test]
fn validated_receive_notice_does_not_break_metadata_or_hide_invalid_reply() {
    assert!(pdu_mode("AT+CMGF?\n+CMTI: \"ME\",1\n+CMGF:0\nOK"));
    assert!(pdu_mode("+CMTI: \"ME\",1\nAT+CMGF?\n+CMGF:0\nOK"));
    assert!(storage("+CPMS: \"SM\",0,50,\"SM\",0,50\n+CMTI: \"SM\",2\nOK").is_ok());
    assert!(wlmb_server::sms_session::text_ready(
        "+CSCS: \"UCS2\"\n+CMTI: \"ME\",1\nOK",
        "+CSDH:1\nOK"
    ));
    for hint in [
        "+CMTI: \"ME\",65536",
        "+CMTI: \"ME\",1,2",
        "+CMTI: ME,1",
        "+CME ERROR: 4",
        "OK",
    ] {
        assert!(!pdu_mode(&format!("+CMGF:0\n{hint}\nOK")));
    }
    assert!(send_reference("+CMGS:7\n+CMTI: \"ME\",1\nOK").is_err());
}
#[test]
fn invalid_metadata_recovers_once_without_replaying_message_operations() {
    let mut calls = 0;
    let s = wlmb_server::sms_session::storage_query(|| {
        calls += 1;
        Ok(if calls == 1 {
            "OK\r\n".into()
        } else {
            "+CPMS: \"SM\",0,50,\"SM\",0,50,\"SM\",0,50\r\nOK\r\n".into()
        })
    })
    .unwrap();
    assert_eq!(s.read, "SM");
    assert_eq!(calls, 2);
    calls = 0;
    assert!(
        wlmb_server::sms_session::storage_query(|| {
            calls += 1;
            Err(wlmb_server::PanelError::new("AT_FAILED", 503))
        })
        .is_err()
    );
    assert_eq!(calls, 1, "Uncertain transport errors are never replayed");
}
#[test]
fn storage_requires_complete_unambiguous_selection_and_valid_counters() {
    let raw = "+CPMS: \"ME\",1,100,\"SM\",0,20,\"MT\",1,120\r\nOK\r\n";
    let before = storage(raw).unwrap();
    assert_eq!(before.read, "ME");
    assert!(before == storage(&raw.replace(",1,100", ",2,100")).unwrap());
    assert!(before != storage(&raw.replace("\"SM\"", "\"ME\"")).unwrap());
    assert!(storage("AT+CPMS?\r\n+CPMS: \"TA\",0,10,\"SM\",0,20\r\nOK\r\n").is_ok());
    for bad in [
        raw.replace("OK", ""),
        raw.replace("OK", "ERROR\r\nOK"),
        raw.replace("OK", "OK\r\nOK"),
        raw.replace("OK", "+CPMS: \"ME\",1,100\r\nOK"),
        raw.replace(",1,100", ",101,100"),
        raw.replace(",1,100", ",-1,100"),
        raw.replace(",1,100", ",+1,100"),
        raw.replace(",1,100", ",1,4294967296"),
        raw.replace("\"ME\"", "\"BM\""),
        "+CPMS: \"ME\"\r\nOK".into(),
        "+CPMS: \"ME\",1,100,\"SM\",0\r\nOK".into(),
    ] {
        assert!(storage(&bad).is_err(), "invalid storage accepted");
    }
}
#[test]
fn mode_and_send_acknowledgement_do_not_accept_an_earlier_ok_or_duplicate_result() {
    assert!(pdu_mode("AT+CMGF?\r\n+CMGF:0\r\nOK\r\n"));
    for raw in [
        "+CMGF:0\nOK\n+CMGF:1",
        "+CMGF:0\nOK\nOK",
        "+CMGF:0\n+CME ERROR:4\nOK",
    ] {
        assert!(!pdu_mode(raw));
    }
    assert_eq!(send_reference("+CMGS: 255\r\nOK\r\n").unwrap(), 255);
    assert_eq!(send_reference("+CMGS: 1,\"0000aB\"\r\nOK\r\n").unwrap(), 1);
    for raw in [
        "+CMGS: 1",
        "+CMGS:1\n+CMGS:2\nOK",
        "+CMGS:256\nOK",
        "+CMGS:-1\nOK",
        "+CMGS:+1\nOK",
        "+CMGS:1,0000\nOK",
        "+CMGS:1,\"0\"\nOK",
        "+CMGS:1,\"GG\"\nOK",
        "+CMGS:1,\"00\",\"00\"\nOK",
        "+CMGS:1\nERROR\nOK",
        "+CMGS:1\nOK\nother",
    ] {
        assert_eq!(send_reference(raw).err().unwrap().code, "SMS_SEND_UNKNOWN");
    }
}
