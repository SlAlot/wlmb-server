#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use wlmb_server::{
    PanelError, Result,
    independent_at::{Endpoint, Hardware, IndependentSource, identify_reply},
    sampler::Source,
};
struct Port {
    hints: Arc<Mutex<wlmb_server::sms_notice::Hints>>,
    calls: Arc<Mutex<Vec<String>>>,
    identity: String,
    responses: Arc<Mutex<BTreeMap<String, Result<String>>>>,
    rows: Arc<Mutex<Vec<Value>>>,
    owners: Arc<Mutex<Vec<String>>>,
    query_race: Arc<Mutex<Option<&'static str>>>,
}
impl Endpoint for Port {
    fn take_sms_hints(&mut self) -> wlmb_server::sms_notice::Hints {
        std::mem::take(&mut *self.hints.lock().unwrap())
    }
    fn query(&mut self, c: &str, _: Duration) -> Result<String> {
        self.calls.lock().unwrap().push(c.into());
        if c == "AT+CMGF?" {
            match self.query_race.lock().unwrap().take() {
                Some("instance") => self.rows.lock().unwrap()[0]["instance"] = json!("1:2"),
                Some("owner") => self.owners.lock().unwrap().push("dialer".into()),
                _ => {}
            }
        }
        if let Some(reply) = self.responses.lock().unwrap().get(c) {
            return reply.clone();
        }
        Ok(match c {
            "AT+CGMM" => self.identity.clone(),
            "AT+CMGF?" => "+CMGF: 0\r\nOK\r\n".into(),
            _ => "OK\r\n".into(),
        })
    }
    fn send_pdu(&mut self, _: &str, _: usize) -> Result<String> {
        self.calls.lock().unwrap().push("send-pdu".into());
        Ok("+CMGS: 1\r\nOK\r\n".into())
    }
    fn quarantined(&self) -> bool {
        false
    }
}
struct Device {
    hints: Arc<Mutex<wlmb_server::sms_notice::Hints>>,
    rows: Arc<Mutex<Vec<Value>>>,
    owners: Arc<Mutex<Vec<String>>>,
    opens: AtomicUsize,
    calls: Arc<Mutex<Vec<String>>>,
    identity: String,
    race: bool,
    responses: Arc<Mutex<BTreeMap<String, Result<String>>>>,
    query_race: Arc<Mutex<Option<&'static str>>>,
}
impl Hardware for Device {
    fn inventory(&self) -> Result<Vec<Value>> {
        Ok(self.rows.lock().unwrap().clone())
    }
    fn owners(&self, _: &str) -> Result<Vec<String>> {
        Ok(self.owners.lock().unwrap().clone())
    }
    fn open(&self, _: &str) -> Result<Box<dyn Endpoint>> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        if self.race {
            self.owners.lock().unwrap().push("dialer".into());
        }
        Ok(Box::new(Port {
            hints: self.hints.clone(),
            calls: self.calls.clone(),
            identity: self.identity.clone(),
            responses: self.responses.clone(),
            rows: self.rows.clone(),
            owners: self.owners.clone(),
            query_race: self.query_race.clone(),
        }))
    }
}
fn setup(owners: Vec<String>, identity: &str, race: bool) -> (Arc<Device>, Arc<IndependentSource>) {
    let device = Arc::new(Device {
        hints: Arc::new(Mutex::new(wlmb_server::sms_notice::Hints::default())),
        rows: Arc::new(Mutex::new(vec![
            json!({"id":"m1","port":"/dev/ttyUSB1","model":"mt5700m-cn","instance":"1:1"}),
        ])),
        owners: Arc::new(Mutex::new(owners)),
        opens: AtomicUsize::new(0),
        calls: Arc::new(Mutex::new(vec![])),
        identity: identity.into(),
        race,
        responses: Arc::new(Mutex::new(BTreeMap::new())),
        query_race: Arc::new(Mutex::new(None)),
    });
    (device.clone(), IndependentSource::new(device))
}
fn pdu_fixture(content: &str, part: Option<u8>) -> (String, usize) {
    let mut data = vec![];
    if let Some(part) = part {
        data.extend_from_slice(&[5, 0, 3, 9, 2, part]);
    }
    for unit in content.encode_utf16() {
        data.extend_from_slice(&unit.to_be_bytes());
    }
    let mut raw = vec![
        0,
        if part.is_some() { 0x40 } else { 0 },
        5,
        0x81,
        0,
        0x80,
        0xf6,
        0,
        8,
        0x62,
        0x01,
        0x01,
        0x21,
        0x43,
        0x65,
        0x23,
        data.len() as u8,
    ];
    raw.extend(data);
    (
        raw.iter().map(|b| format!("{b:02X}")).collect(),
        raw.len() - 1,
    )
}
fn list_fixture(rows: &[(u32, &str, Option<u8>)]) -> String {
    let mut raw = String::new();
    for (index, content, part) in rows {
        let (pdu, n) = pdu_fixture(content, *part);
        raw.push_str(&format!("+CMGL: {index},0,,{n}\n{pdu}\n"));
    }
    raw.push_str("OK\n");
    raw
}
fn seed_pdu(device: &Device, source: &IndependentSource, rows: &[(u32, &str, Option<u8>)]) {
    use wlmb_server::sms_runtime::SmsDevice;
    source.modems().unwrap();
    let mut responses = device.responses.lock().unwrap();
    responses.insert(
        "AT+CPMS?".into(),
        Ok("+CPMS: \"ME\",1,20,\"ME\",1,20,\"ME\",1,20\nOK".into()),
    );
    responses.insert("AT+CMGL=4".into(), Ok(list_fixture(rows)));
    drop(responses);
    source.read("m1").unwrap();
    device.calls.lock().unwrap().clear();
}
fn hint_pdu(device: &Device, index: u32, content: &str, part: Option<u8>) {
    let (pdu, n) = pdu_fixture(content, part);
    device.responses.lock().unwrap().insert(
        format!("AT+CMGR={index}"),
        Ok(format!("+CMGR: 0,,{n}\n{pdu}\nOK")),
    );
    device
        .hints
        .lock()
        .unwrap()
        .observe(&format!("+CMTI: \"ME\",{index}"));
}
#[test]
fn independent_event_reads_only_new_indexes_and_reassembles_raw_parts() {
    use wlmb_server::sms_runtime::SmsDevice;
    let (device, source) = setup(vec![], "MT5700M-CN\nOK", false);
    seed_pdu(&device, &source, &[(1, "旧短信", None), (2, "你", Some(1))]);
    hint_pdu(&device, 3, "好", Some(2));
    let records = source.read_event("m1").unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].message.content, "你好");
    assert_eq!(records[1].indexes, [2, 3]);
    assert!(records[1].concatenation.is_none());
    assert_eq!(
        device
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with("AT+CMGR="))
            .count(),
        1
    );
    assert!(
        !device
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c == "AT+CMGL=4")
    );
    hint_pdu(&device, 3, "好", Some(2));
    assert_eq!(source.read_event("m1").unwrap()[1].message.content, "你好");
    hint_pdu(&device, 1, "新短信", None);
    assert_eq!(
        source.read_event("m1").unwrap()[0].message.content,
        "新短信"
    );
}
#[test]
fn lost_overflow_wrong_storage_and_cancelled_cache_fall_back_without_switching_storage() {
    use wlmb_server::sms_runtime::SmsDevice;
    for reason in [
        "no-hint",
        "overflow",
        "wrong-store",
        "backlog",
        "stop",
        "storage-change",
    ] {
        let (device, source) = setup(vec![], "MT5700M-CN\nOK", false);
        seed_pdu(&device, &source, &[(1, "旧短信", None)]);
        hint_pdu(&device, 2, "新短信", None);
        match reason {
            "no-hint" => *device.hints.lock().unwrap() = Default::default(),
            "overflow" => device.hints.lock().unwrap().overflow = true,
            "wrong-store" => {
                let mut hints = device.hints.lock().unwrap();
                hints.indexes.clear();
                hints.observe("+CMTI: \"SM\",2");
            }
            "backlog" => {
                for i in 3..12 {
                    hint_pdu(&device, i, "积压", None);
                }
            }
            "stop" => source.stop_notices(),
            "storage-change" => {
                device.responses.lock().unwrap().insert(
                    "AT+CPMS?".into(),
                    Ok("+CPMS: \"SM\",1,20,\"SM\",1,20,\"SM\",1,20\nOK".into()),
                );
            }
            _ => {}
        }
        source.read_event("m1").unwrap();
        let calls = device.calls.lock().unwrap();
        assert!(calls.iter().any(|c| c == "AT+CMGL=4"), "{reason}");
        assert!(
            !calls.iter().any(|c| c.starts_with("AT+CMGR=")
                || c.starts_with("AT+CPMS=")
                || c.starts_with("AT+CMGF=")),
            "{reason}"
        );
    }
}
#[test]
fn unsupported_or_ambiguous_index_read_is_not_repeated_on_same_identity() {
    use wlmb_server::sms_runtime::SmsDevice;
    for reply in [
        Err(PanelError::new("AT_UNSUPPORTED", 422)),
        Ok("+CMGR: incomplete\nOK".into()),
    ] {
        let (device, source) = setup(vec![], "MT5700M-CN\nOK", false);
        seed_pdu(&device, &source, &[(1, "旧短信", None)]);
        hint_pdu(&device, 2, "新短信", None);
        device
            .responses
            .lock()
            .unwrap()
            .insert("AT+CMGR=2".into(), reply);
        source.read_event("m1").unwrap();
        device.calls.lock().unwrap().clear();
        hint_pdu(&device, 2, "新短信", None);
        source.read_event("m1").unwrap();
        assert!(
            !device
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c == "AT+CMGR=2")
        );
        assert!(
            device
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c == "AT+CMGL=4")
        );
    }
}
#[test]
fn incremental_read_identity_and_ownership_races_fail_closed() {
    use wlmb_server::sms_runtime::SmsDevice;
    for (race, code) in [("instance", "MODEM_CHANGED"), ("owner", "AT_PORT_CONFLICT")] {
        let (device, source) = setup(vec![], "MT5700M-CN\nOK", false);
        seed_pdu(&device, &source, &[(1, "旧短信", None)]);
        hint_pdu(&device, 2, "新短信", None);
        *device.query_race.lock().unwrap() = Some(race);
        assert_eq!(source.read_event("m1").err().unwrap().code, code);
        assert!(
            !device
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.starts_with("AT+CMGR=") || c == "AT+CMGL=4")
        );
    }
}
#[test]
fn indexed_receive_path_is_shared_by_every_catalog_model_not_only_mt5700() {
    use wlmb_server::sms_runtime::SmsDevice;
    for (_, models) in wlmb_server::catalog::MODELS {
        for model in models.split_whitespace() {
            let (device, source) = setup(
                vec![],
                &format!("{}\nOK", model.to_ascii_uppercase()),
                false,
            );
            device.rows.lock().unwrap()[0]["model"] = json!(model);
            seed_pdu(&device, &source, &[(1, "旧短信", None)]);
            hint_pdu(&device, 2, "新短信", None);
            let records = source.read_event("m1").unwrap();
            assert_eq!(records.len(), 2, "{model}");
            assert!(
                !device
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|c| c == "AT+CMGL=4"),
                "{model}"
            );
        }
    }
}
#[test]
fn frequent_events_never_extend_full_snapshot_validation_deadline() {
    use wlmb_server::sms_runtime::SmsDevice;
    let (device, source) = setup(vec![], "MT5700M-CN\nOK", false);
    seed_pdu(&device, &source, &[(1, "旧短信", None)]);
    hint_pdu(&device, 2, "新短信", None);
    source.read_event("m1").unwrap();
    std::thread::sleep(Duration::from_millis(10050));
    device.calls.lock().unwrap().clear();
    hint_pdu(&device, 3, "另一短信", None);
    source.read_event("m1").unwrap();
    let calls = device.calls.lock().unwrap();
    assert!(calls.iter().any(|c| c == "AT+CMGL=4"));
    assert!(!calls.iter().any(|c| c == "AT+CMGR=3"));
}
#[test]
fn unplugged_modem_cache_cannot_exhaust_the_live_modem_budget() {
    use wlmb_server::sms_runtime::SmsDevice;
    let (device, source) = setup(vec![], "MT5700M-CN\nOK", false);
    device.responses.lock().unwrap().insert(
        "AT^MONSC".into(),
        Ok("^MONSC: fixture\n+CMTI: \"ME\",2\nOK".into()),
    );
    device.responses.lock().unwrap().insert(
        "AT+CPMS?".into(),
        Ok("+CPMS: \"ME\",1,20,\"ME\",1,20,\"ME\",1,20\nOK".into()),
    );
    device
        .responses
        .lock()
        .unwrap()
        .insert("AT+CMGL=4".into(), Ok(list_fixture(&[(1, "短信", None)])));
    for i in 0..24 {
        let id = format!("hotplug-{i}");
        device.rows.lock().unwrap()[0]["id"] = json!(id);
        device.rows.lock().unwrap()[0]["instance"] = json!(format!("1:{i}"));
        source.modems().unwrap();
        assert_eq!(source.read(&id).unwrap()[0].message.modem, id);
        source
            .query(&id, "AT^MONSC", "/dev/ttyUSB1", "mt5700m-cn")
            .unwrap();
        assert!(source.notice_generation(&id) > 0);
    }
}
#[test]
fn identity_is_exact_and_never_brand_or_substring_guess() {
    assert_eq!(
        identify_reply("+CGMM: MT5700M-CN\r\nOK\r\n").unwrap()["model"],
        "mt5700m-cn"
    );
    assert!(identify_reply("firmware with MT5700M-CN support\r\nOK\r\n").is_none());
    assert!(identify_reply("MT5700M-CN\r\nRM520N-GL\r\nOK\r\n").is_none());
    assert!(identify_reply("TDTech\r\nOK\r\n").is_none());
    assert!(identify_reply("MT5700M-CN!\r\nOK\r\n").is_none());
    assert!(identify_reply("MT5700M-CN").is_none());
}
#[test]
fn sms_send_accepts_portable_mode_spacing_but_never_sends_after_ambiguous_mode() {
    use wlmb_server::sms_runtime::SmsDevice;
    let (device, source) = setup(vec![], "MT5700M-CN\r\nOK\r\n", false);
    source.modems().unwrap();
    device
        .responses
        .lock()
        .unwrap()
        .insert("AT+CMGF?".into(), Ok("+CMGF:0\r\nOK\r\n".into()));
    assert_eq!(source.send("m1", "10086", "fixture").unwrap()["sent"], true);
    device.calls.lock().unwrap().clear();
    for raw in ["+CMGF:0\nOK\n+CMGF:1", "+CMGF:0\nOK\nOK", "+CMGF:0"] {
        device
            .responses
            .lock()
            .unwrap()
            .insert("AT+CMGF?".into(), Ok(raw.into()));
        assert_eq!(
            source.send("m1", "10086", "fixture").err().unwrap().code,
            "SMS_PDU_MODE_REQUIRED"
        );
    }
    assert!(!device.calls.lock().unwrap().iter().any(|s| s == "send-pdu"));
}
#[test]
fn hotplug_or_new_owner_during_mode_check_prevents_paid_send_and_stale_read() {
    use wlmb_server::sms_runtime::SmsDevice;
    for (race, code) in [("instance", "MODEM_CHANGED"), ("owner", "AT_PORT_CONFLICT")] {
        let (device, source) = setup(vec![], "MT5700M-CN\r\nOK\r\n", false);
        source.modems().unwrap();
        *device.query_race.lock().unwrap() = Some(race);
        assert_eq!(
            source.send("m1", "10086", "fixture").unwrap_err().code,
            code
        );
        assert!(!device.calls.lock().unwrap().iter().any(|s| s == "send-pdu"));
    }
    let (device, source) = setup(vec![], "MT5700M-CN\r\nOK\r\n", false);
    source.modems().unwrap();
    *device.query_race.lock().unwrap() = Some("instance");
    assert_eq!(source.read("m1").err().unwrap().code, "MODEM_CHANGED");
    assert!(
        !device
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("AT+CMGL="))
    );
}
#[test]
fn dedicated_text_read_checks_metadata_without_switching_storage_or_encoding() {
    use wlmb_server::sms_runtime::SmsDevice;
    let (device, source) = setup(vec![], "MT5700M-CN\r\nOK\r\n", false);
    source.modems().unwrap();
    let mut responses = device.responses.lock().unwrap();
    for (cmd, raw) in [
        ("AT+CMGF?", "+CMGF:1\nOK"),
        ("AT+CSCS?", "+CSCS: \"UCS2\"\nOK"),
        ("AT+CSDH?", "+CSDH:1\nOK"),
        ("AT+CPMS?", "+CPMS: \"ME\",1,20,\"ME\",1,20,\"ME\",1,20\nOK"),
        (
            "AT+CMGL=\"ALL\"",
            "+CMGL: 1,\"REC UNREAD\",\"00310030003000380036\",\"\",\"26/10/11,12:34:56+32\",129,4\n4F60\nOK",
        ),
        (
            "AT+CMGR=1",
            "+CMGR: \"REC READ\",\"00310030003000380036\",\"\",\"26/10/11,12:34:56+32\",129,0,0,8,\"0031\",129,4\n4F60\nOK",
        ),
    ] {
        responses.insert(cmd.into(), Ok(raw.into()));
    }
    drop(responses);
    let rows = source.read("m1").unwrap();
    assert_eq!(rows[0].message.content, "你");
    assert_eq!(rows[0].dcs, 8);
    assert!(
        !device
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("AT+CMGF=")
                || s.starts_with("AT+CSCS=")
                || s.starts_with("AT+CPMS="))
    );
}
#[test]
fn conflict_release_and_dedicated_sampler_are_not_gated_on_legacy_shared_flag() {
    let (device, source) = setup(vec!["dialer".into()], "MT5700M-CN\r\nOK\r\n", false);
    source.modems().unwrap();
    device.owners.lock().unwrap().clear();
    assert_eq!(source.modems().unwrap()[0]["channelReady"], true);
    let sampler = wlmb_server::sampler::Sampler::new(source);
    sampler.tick().unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(1);
    while std::time::Instant::now() < until
        && !device.calls.lock().unwrap().iter().any(|c| c == "AT^MONSC")
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(device.calls.lock().unwrap().iter().any(|c| c == "AT^MONSC"));
    sampler.stop(Duration::from_secs(1)).unwrap();
}
#[test]
fn sms_current_storage_read_and_delete_check_real_message_before_write() {
    use wlmb_server::sms_runtime::SmsDevice;
    let (device, source) = setup(vec![], "MT5700M-CN\r\nOK\r\n", false);
    source.modems().unwrap();
    let pdu = wlmb_server::sms_pdu::encode_submit("10086", "fixture").unwrap();
    device.responses.lock().unwrap().insert(
        "AT+CPMS?".into(),
        Ok("+CPMS: \"ME\",1,20,\"ME\",1,20,\"ME\",1,20\r\nOK\r\n".into()),
    );
    device.responses.lock().unwrap().insert(
        "AT+CMGL=4".into(),
        Ok(format!(
            "+CMGL: 7,3,,{}\r\n{}\r\nOK\r\n",
            pdu.tpdu_bytes, pdu.hex
        )),
    );
    let rows = source.read("m1").unwrap();
    let key = rows[0].message.fingerprint().unwrap();
    assert_eq!(rows[0].message.storage, "ME");
    assert_eq!(rows[0].message.index, 7);
    assert_eq!(
        source.delete("m1", &"a".repeat(64)).err().unwrap().code,
        "SMS_CHANGED"
    );
    assert!(
        !device
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("AT+CMGD="))
    );
    for raw in ["OK\nOK", "ERROR\nOK", "", "OK\n+CMTI: \"ME\",8"] {
        device
            .responses
            .lock()
            .unwrap()
            .insert("AT+CMGD=7,0".into(), Ok(raw.into()));
        assert_eq!(
            source.delete("m1", &key).unwrap_err().code,
            "SMS_DELETE_UNKNOWN"
        );
    }
    device.calls.lock().unwrap().clear();
    device.responses.lock().unwrap().remove("AT+CMGD=7,0");
    source.delete("m1", &key).unwrap();
    assert_eq!(
        device
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|s| *s == "AT+CMGD=7,0")
            .count(),
        1
    );
    let revision = source.revision("m1").unwrap();
    assert_eq!(revision.len(), 64);
    device.rows.lock().unwrap()[0]["instance"] = json!("1:2");
    assert_eq!(
        source.send("m1", "10086", "test").err().unwrap().code,
        "MODEM_CHANGED"
    );
    assert!(!device.calls.lock().unwrap().iter().any(|s| s == "send-pdu"));
}
#[test]
fn busy_endpoint_never_opens_or_sends_and_post_open_race_never_sends() {
    let (device, source) = setup(vec!["dialer".into()], "MT5700M-CN\r\nOK\r\n", false);
    assert_eq!(source.modems().unwrap()[0]["channelReady"], false);
    assert_eq!(device.opens.load(Ordering::SeqCst), 0);
    assert!(device.calls.lock().unwrap().is_empty());
    let (device, source) = setup(vec![], "MT5700M-CN\r\nOK\r\n", true);
    assert_eq!(source.modems().unwrap()[0]["channelReady"], false);
    assert_eq!(device.opens.load(Ordering::SeqCst), 1);
    assert!(device.calls.lock().unwrap().is_empty());
}
#[test]
fn verified_identity_is_cached_and_sms_mode_and_send_share_one_endpoint() {
    let (device, source) = setup(vec![], "MT5700M-CN\r\nOK\r\n", false);
    let rows = source.modems().unwrap();
    assert_eq!(rows[0]["channelReady"], true);
    assert_eq!(rows[0]["transport"], "dedicated");
    source.modems().unwrap();
    assert_eq!(device.opens.load(Ordering::SeqCst), 1);
    source
        .query("m1", "AT^MONSC", "/dev/ttyUSB1", "mt5700m-cn")
        .unwrap();
    source.send_sms_pdu("m1", "00010203", 3).unwrap();
    assert_eq!(
        *device.calls.lock().unwrap(),
        vec!["AT+CGMM", "AT^MONSC", "AT+CMGF?", "send-pdu"]
    );
    assert_eq!(
        source
            .query("m1", "AT^MONSC", "/dev/ttyUSB9", "mt5700m-cn")
            .unwrap_err()
            .code,
        "MODEM_CHANGED"
    );
}
#[test]
fn identity_mismatch_and_new_external_owner_fail_closed() {
    let (device, source) = setup(vec![], "RM520N-GL\r\nOK\r\n", false);
    assert_eq!(source.modems().unwrap()[0]["channelReady"], false);
    assert_eq!(device.opens.load(Ordering::SeqCst), 1);
    assert!(
        source
            .query("m1", "AT^MONSC", "/dev/ttyUSB1", "mt5700m-cn")
            .is_err()
    );
    let (device, source) = setup(vec![], "MT5700M-CN\r\nOK\r\n", false);
    source.modems().unwrap();
    device
        .owners
        .lock()
        .unwrap()
        .push("other-wlmb-server".into());
    assert_eq!(
        source
            .query("m1", "AT^MONSC", "/dev/ttyUSB1", "mt5700m-cn")
            .unwrap_err()
            .code,
        "AT_PORT_CONFLICT"
    );
    assert_eq!(*device.calls.lock().unwrap(), vec!["AT+CGMM"]);
}
#[test]
fn hotplug_revalidates_identity_and_duplicate_ports_do_not_open() {
    let (device, source) = setup(vec![], "MT5700M-CN\r\nOK\r\n", false);
    source.modems().unwrap();
    device.rows.lock().unwrap()[0]["instance"] = json!("1:2");
    source.modems().unwrap();
    assert_eq!(device.opens.load(Ordering::SeqCst), 2);
    let mut duplicate = device.rows.lock().unwrap()[0].clone();
    duplicate["id"] = json!("m2");
    device.rows.lock().unwrap().push(duplicate);
    let e: PanelError = source.modems().unwrap_err();
    assert_eq!(e.code, "INVALID_MODEM_CONFIG");
    assert_eq!(device.opens.load(Ordering::SeqCst), 2);
}
