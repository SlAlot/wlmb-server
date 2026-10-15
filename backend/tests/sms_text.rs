#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use std::collections::BTreeMap;
use wlmb_server::{
    PanelError,
    sms::Direction,
    sms_text::{self, Cache},
};
fn hex(raw: &str) -> String {
    raw.encode_utf16().map(|u| format!("{u:04X}")).collect()
}
fn listing(index: u32, contact: &str, content: &str) -> String {
    let body = hex(content);
    format!(
        "+CMGL: {index},\"REC UNREAD\",\"{}\",\"\",\"26/10/11,12:34:56+32\",129,{}\r\n{body}\r\n",
        hex(contact),
        body.len()
    )
}
fn detail(index: u32, contact: &str, content: &str, dcs: u8, fo: u8) -> String {
    let body = hex(content);
    format!(
        "AT+CMGR={index}\r\n+CMGR: \"REC READ\",\"{}\",\"\",\"26/10/11,12:34:56+32\",129,{fo},0,{dcs},\"00310030003000380036\",129,{}\r\n{body}\r\nOK\r\n",
        hex(contact),
        body.len()
    )
}
struct Device {
    raw: String,
    details: BTreeMap<String, String>,
    commands: Vec<String>,
    changed: bool,
    stores: u32,
}
impl Device {
    fn query(&mut self, cmd: &str, seconds: u64) -> wlmb_server::Result<String> {
        self.commands.push(cmd.into());
        if cmd.starts_with("AT+CMGR=") {
            assert_eq!(seconds, 1);
        }
        Ok(match cmd {
            "AT+CSCS?" => "+CSCS: \"UCS2\"\r\nOK\r\n".into(),
            "AT+CSDH?" => "+CSDH: 1\r\nOK\r\n".into(),
            "AT+CMGF?" => "+CMGF: 1\r\nOK\r\n".into(),
            "AT+CPMS?" => {
                self.stores += 1;
                format!(
                    "+CPMS: \"{}\",2,100,\"ME\",2,100,\"ME\",2,100\r\nOK\r\n",
                    if self.changed && self.stores % 2 == 0 {
                        "SM"
                    } else {
                        "ME"
                    }
                )
            }
            "AT+CMGL=\"ALL\"" => self.raw.clone(),
            other => self
                .details
                .get(other)
                .ok_or_else(|| PanelError::new("UNEXPECTED_QUERY", 422))?
                .clone(),
        })
    }
}
fn device() -> Device {
    Device {
        raw: format!(
            "{}{}OK\r\n",
            listing(1, "10086", "你好"),
            listing(2, "10010", "Balance €")
        ),
        details: BTreeMap::from([
            ("AT+CMGR=1".into(), detail(1, "10086", "你好", 8, 0)),
            ("AT+CMGR=2".into(), detail(2, "10010", "Balance €", 0, 0)),
        ]),
        commands: vec![],
        changed: false,
        stores: 0,
    }
}
#[test]
fn text_uses_verified_headers_and_reuses_only_private_metadata_hashes() {
    let mut d = device();
    let mut cache = Cache::default();
    let rows = sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].message.content, "你好");
    assert_eq!(rows[0].message.contact, "10086");
    assert_eq!(rows[0].message.timestamp, "2026-10-11 12:34:56 +08:00");
    assert!(rows[0].message.direction == Direction::Incoming);
    assert_eq!(rows[1].dcs, 0);
    assert!(rows.iter().all(|r| r.concatenation.is_none()));
    d.commands.clear();
    sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache).unwrap();
    assert!(!d.commands.iter().any(|s| s.starts_with("AT+CMGR=")));
    assert!(
        !d.commands
            .iter()
            .any(|s| s == "AT+CMGF=0" || s.starts_with("AT+CPMS="))
    );
}
#[test]
fn mode_or_charset_is_not_changed_and_unverified_body_is_never_forwardable() {
    for replacement in ["+CSCS: \"IRA\"\nOK", "+CSCS: \"UCS2\"\nERROR\nOK"] {
        let mut d = device();
        let mut cache = Cache::default();
        let e = sms_text::read(
            |cmd, timeout| {
                if cmd == "AT+CSCS?" {
                    Ok(replacement.into())
                } else {
                    d.query(cmd, timeout)
                }
            },
            "m1",
            &mut cache,
        )
        .err()
        .unwrap();
        assert_eq!(e.code, "SMS_TEXT_SETTINGS_REQUIRED");
        assert!(!d.commands.iter().any(|s| s.starts_with("AT+CMGL=")));
    }
    for (dcs, fo) in [(4, 0), (8, 64), (0, 1), (32, 0)] {
        let mut d = device();
        d.details
            .insert("AT+CMGR=1".into(), detail(1, "10086", "你好", dcs, fo));
        assert!(
            sms_text::read(
                |cmd, timeout| d.query(cmd, timeout),
                "m1",
                &mut Cache::default()
            )
            .is_err()
        );
    }
}
#[test]
fn pending_verification_is_bounded_and_never_publishes_a_partial_inbox() {
    let mut d = device();
    d.raw = String::new();
    d.details.clear();
    for n in 1..=12 {
        d.raw += &listing(n, "10086", "你好");
        d.details
            .insert(format!("AT+CMGR={n}"), detail(n, "10086", "你好", 8, 0));
    }
    d.raw += "OK\r\n";
    let mut cache = Cache::default();
    assert_eq!(
        sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache)
            .err()
            .unwrap()
            .code,
        "SMS_TEXT_VERIFY_PENDING"
    );
    assert_eq!(
        d.commands
            .iter()
            .filter(|s| s.starts_with("AT+CMGR="))
            .count(),
        8
    );
    d.commands.clear();
    let rows = sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache).unwrap();
    assert_eq!(rows.len(), 12);
    assert_eq!(
        d.commands
            .iter()
            .filter(|s| s.starts_with("AT+CMGR="))
            .count(),
        4
    );
    cache.bind("replacement".into());
    d.commands.clear();
    assert_eq!(
        sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache)
            .err()
            .unwrap()
            .code,
        "SMS_TEXT_VERIFY_PENDING"
    );
    assert_eq!(
        d.commands
            .iter()
            .filter(|s| s.starts_with("AT+CMGR="))
            .count(),
        8
    );
}
#[test]
fn new_content_at_same_index_cannot_reuse_old_dcs_validation() {
    let mut d = device();
    let mut cache = Cache::default();
    sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache).unwrap();
    d.raw = format!("{}OK\r\n", listing(1, "10086", "更新"));
    d.commands.clear();
    assert!(sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache).is_err());
    assert!(d.commands.contains(&"AT+CMGR=1".into()));
}
#[test]
fn changed_address_type_at_same_index_requires_new_metadata() {
    let mut d = device();
    let mut cache = Cache::default();
    sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache).unwrap();
    d.raw = d.raw.replace(",129,", ",145,");
    d.commands.clear();
    assert!(sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache).is_err());
    assert!(d.commands.contains(&"AT+CMGR=1".into()));
}
#[test]
fn storage_race_clears_metadata_even_when_all_slots_were_already_cached() {
    let mut d = device();
    let mut cache = Cache::default();
    sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache).unwrap();
    d.changed = true;
    assert_eq!(
        sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache)
            .err()
            .unwrap()
            .code,
        "SMS_STORAGE_CHANGED"
    );
    d.changed = false;
    d.commands.clear();
    sms_text::read(|cmd, timeout| d.query(cmd, timeout), "m1", &mut cache).unwrap();
    assert_eq!(
        d.commands
            .iter()
            .filter(|s| s.starts_with("AT+CMGR="))
            .count(),
        2
    );
}
#[test]
fn malformed_hex_duplicate_slots_fake_lengths_and_status_reports_fail_closed() {
    let mut d = device();
    let good = d.raw.clone();
    for raw in [
        good.replace("OK", ""),
        good.replace("OK", "ERROR\nOK"),
        good.replace("+CMGL: 2", "+CMGL: 1"),
        good.replace("4F60597D", "D800597D"),
        good.replace(",129,8", ",129,7"),
        good.replace("26/10/11", "26/02/30"),
        good.replace("\"REC UNREAD\"", "\"STO UNSENT\""),
        good.replace("\"00310030003000380036\"", "17"),
        good.replace(
            "4F60597D",
            "OK\n+CMGL: 9,\"REC READ\",\"0031\",\"\",\"26/10/11,12:34:56+32\",129,4",
        ),
    ] {
        d.raw = raw;
        assert!(
            sms_text::read(
                |cmd, timeout| d.query(cmd, timeout),
                "m1",
                &mut Cache::default()
            )
            .is_err()
        );
    }
}
#[test]
fn stored_sent_text_has_no_invented_timestamp_and_requires_submit_first_octet() {
    let mut d = device();
    let body = hex("Hi");
    d.raw = format!("+CMGL: 7,\"STO SENT\",\"00310030003000380036\",\"\",129,8\n{body}\nOK\n");
    d.details.insert("AT+CMGR=7".into(),format!("+CMGR: \"STO SENT\",\"00310030003000380036\",\"\",129,1,0,0,\"0031\",129,8\n{body}\nOK\n"));
    let rows = sms_text::read(
        |cmd, timeout| d.query(cmd, timeout),
        "m1",
        &mut Cache::default(),
    )
    .unwrap();
    assert!(rows[0].message.direction == Direction::Outgoing);
    assert_eq!(rows[0].message.timestamp, "");
    d.details.insert("AT+CMGR=7".into(),format!("+CMGR: \"STO SENT\",\"00310030003000380036\",\"\",129,17,0,0,167,\"0031\",129,8\n{body}\nOK\n"));
    assert!(
        sms_text::read(
            |cmd, timeout| d.query(cmd, timeout),
            "m1",
            &mut Cache::default()
        )
        .is_ok()
    );
    for raw in [
        format!(
            "+CMGR: \"STO SENT\",\"00310030003000380036\",\"\",129,1,0,0,167,\"0031\",129,8\n{body}\nOK\n"
        ),
        format!(
            "+CMGR: \"STO SENT\",\"00310030003000380036\",\"\",129,17,0,0,999,\"0031\",129,8\n{body}\nOK\n"
        ),
    ] {
        d.details.insert("AT+CMGR=7".into(), raw);
        assert!(
            sms_text::read(
                |cmd, timeout| d.query(cmd, timeout),
                "m1",
                &mut Cache::default()
            )
            .is_err()
        );
    }
}
