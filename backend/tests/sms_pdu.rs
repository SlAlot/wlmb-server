#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::{
    sms::Direction,
    sms_pdu::{decode, encode_submit, parse_cmgl},
};
fn hex(raw: &[u8]) -> String {
    raw.iter().map(|b| format!("{b:02X}")).collect()
}
#[test]
fn pdu_mode_whitespace_is_portable_but_partial_and_ambiguous_responses_fail() {
    use wlmb_server::sms_pdu::pdu_mode;
    for raw in ["+CMGF:0\r\nOK\r\n", "AT+CMGF?\r\n+CMGF: 0\r\nOK\r\n"] {
        assert!(pdu_mode(raw));
    }
    for raw in [
        "+CMGF:0",
        "OK",
        "+CMGF:1\r\nOK",
        "+CMGF:0\r\n+CMGF:1\r\nOK",
        "+CMGF:0\r\nERROR\r\nOK",
    ] {
        assert!(!pdu_mode(raw));
    }
}
fn deliver(dcs: u8, header: bool, data: &[u8], length: u8) -> Vec<u8> {
    let mut raw = vec![
        0,
        if header { 0x40 } else { 0 },
        5,
        0x81,
        0,
        0x80,
        0xf6,
        0,
        dcs,
        0x62,
        0x01,
        0x01,
        0x21,
        0x43,
        0x65,
        0x23,
        length,
    ];
    raw.extend_from_slice(data);
    raw
}
fn listing(raw: &[u8], index: u32, status: u8) -> String {
    format!(
        "+CMGL: {index},{status},,{}\r\n{}\r\n",
        raw.len() - 1,
        hex(raw)
    )
}
#[test]
fn indexed_pdu_response_accepts_echo_and_notifications_but_rejects_ambiguity() {
    let raw = deliver(8, false, &[0x4f, 0x60], 2);
    let answer = format!(
        "AT+CMGR=7\n+CMGR: 0,\"\",{}\n{}\n+CMTI: \"ME\",8\nOK",
        raw.len() - 1,
        hex(&raw)
    );
    let r = wlmb_server::sms_pdu::parse_cmgr(&answer, "m1", "ME", 7).unwrap();
    assert_eq!(r.message.content, "你");
    assert_eq!(r.message.index, 7);
    for bad in [
        answer.replace("AT+CMGR=7", "AT+CMGR=8"),
        answer.replace("0,\"\",", "0,\","),
        answer.replace("0,\"\",", "5,\"\","),
        answer.replace("\nOK", "\nOK\nOK"),
        answer.replace("\nOK", ""),
        answer.replace(&hex(&raw), "ZZ"),
    ] {
        assert!(wlmb_server::sms_pdu::parse_cmgr(&bad, "m1", "ME", 7).is_err());
    }
}
#[test]
fn ucs2_gsm_sender_timestamp_and_submit_roundtrip() {
    let raw = deliver(8, false, &[0x4f, 0x60, 0x59, 0x7d], 4);
    let r = decode(&hex(&raw), raw.len() - 1, "m1", "ME", 1, 0).unwrap();
    assert_eq!(r.message.contact, "00086");
    assert_eq!(r.message.timestamp, "2026-10-10 12:34:56 +08:00");
    assert_eq!(r.message.content, "你好");
    assert!(r.message.direction == Direction::Incoming);
    let raw = deliver(0, false, &[0xc8, 0xf7, 0x1d, 0x14, 0x06], 5);
    let r = decode(&hex(&raw), raw.len() - 1, "m1", "ME", 1, 1).unwrap();
    assert_eq!(r.message.content, "How a");
    let s = encode_submit("+8613800138000", "你好😀").unwrap();
    let r = decode(&s.hex, s.tpdu_bytes, "m1", "ME", 2, 3).unwrap();
    assert_eq!(r.message.content, "你好😀");
    assert_eq!(r.message.contact, "+8613800138000");
    assert!(r.message.direction == Direction::Outgoing);
}
#[test]
fn multipart_is_complete_ordered_and_never_mix_directions() {
    let a = deliver(8, true, &[5, 0, 3, 9, 2, 1, 0x4f, 0x60], 8);
    let b = deliver(8, true, &[5, 0, 3, 9, 2, 2, 0x59, 0x7d], 8);
    let list = format!("{}{}OK\r\n", listing(&b, 4, 1), listing(&a, 3, 0));
    let rows = parse_cmgl(&list, "m1", "ME").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].message.content, "你好");
    assert_eq!(rows[0].indexes, vec![3, 4]);
    let incomplete = parse_cmgl(&format!("{}OK\r\n", listing(&a, 3, 0)), "m1", "ME").unwrap();
    assert_eq!(incomplete[0].message.content, "你");
    assert_eq!(incomplete[0].concatenation.as_ref().unwrap().total, 2);
    let mixed = parse_cmgl(
        &format!("{}{}OK\r\n", listing(&a, 3, 0), listing(&b, 4, 3)),
        "m1",
        "ME",
    )
    .unwrap();
    assert_eq!(mixed.len(), 2);
    assert!(mixed.iter().all(|r| r.concatenation.is_some()));
    assert!(
        parse_cmgl(
            &format!("{}{}OK\r\n", listing(&a, 3, 0), listing(&a, 4, 0)),
            "m1",
            "ME"
        )
        .is_err()
    );
}
#[test]
fn gsm_udh_padding_and_extensions_decode_without_header_noise() {
    let mut data = vec![5, 0, 3, 9, 2, 1, 0, 0, 0];
    for (i, v) in [b'A', 27, 101].iter().enumerate() {
        for bit in 0..7 {
            if v & (1 << bit) != 0 {
                let at = 49 + i * 7 + bit;
                data[at / 8] |= 1 << (at % 8);
            }
        }
    }
    let raw = deliver(0, true, &data, 10);
    let r = decode(&hex(&raw), raw.len() - 1, "m1", "ME", 1, 0).unwrap();
    assert_eq!(r.message.content, "A€");
}
#[test]
fn incomplete_fragments_are_visible_but_never_forwarded_or_concatenated_across_gaps() {
    let first = deliver(8, true, &[5, 0, 3, 9, 3, 1, 0x4f, 0x60], 8);
    let last = deliver(8, true, &[5, 0, 3, 9, 3, 3, 0x59, 0x7d], 8);
    let normal = deliver(8, false, &[0x4e, 0x00], 2);
    let rows = parse_cmgl(
        &format!(
            "{}{}{}OK\r\n",
            listing(&first, 1, 0),
            listing(&last, 3, 0),
            listing(&normal, 4, 0)
        ),
        "m1",
        "ME",
    )
    .unwrap();
    assert_eq!(rows.len(), 3);
    let forwarded = wlmb_server::sms_runtime::forwardable(&rows);
    assert_eq!(forwarded.len(), 1);
    assert_eq!(forwarded[0].content, "一");
    assert_eq!(rows[1].concatenation.as_ref().unwrap().part, 3);
    let duplicate = format!("{}{}OK\r\n", listing(&first, 1, 0), listing(&first, 2, 0));
    assert!(parse_cmgl(&duplicate, "m1", "ME").is_err());
}
#[test]
fn invalid_lengths_binary_incomplete_snapshot_and_paid_split_rejected() {
    let raw = deliver(8, false, &[0x4f, 0x60], 2);
    assert!(decode(&hex(&raw), raw.len(), "m1", "ME", 1, 0).is_err());
    assert!(parse_cmgl(&listing(&raw, 1, 0), "m1", "ME").is_err());
    assert!(
        parse_cmgl(
            &format!("{}{}OK", listing(&raw, 1, 0), listing(&raw, 1, 0)),
            "m1",
            "ME"
        )
        .is_err()
    );
    let binary = deliver(4, false, &[65], 1);
    assert_eq!(
        decode(&hex(&binary), binary.len() - 1, "m1", "ME", 1, 0)
            .err()
            .unwrap()
            .code,
        "SMS_BINARY_UNSUPPORTED"
    );
    let mut bad = raw.clone();
    bad[11] = 0x92;
    bad[10] = 0x20;
    assert!(decode(&hex(&bad), bad.len() - 1, "m1", "ME", 1, 0).is_err());
    assert!(encode_submit("10086\rAT", "test").is_err());
    assert!(encode_submit("10086", &"字".repeat(71)).is_err());
    assert!(encode_submit("10086", &"字".repeat(70)).is_ok());
}
