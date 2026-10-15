#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{net::Ipv4Addr, time::Duration};
use wlmb_server::{
    PanelError, Result, boundary, controls, radio,
    security::{Security, Ubus, read_only, validate_at},
};

fn at(t: u64) -> Duration {
    Duration::from_secs(t)
}
fn peer() -> Ipv4Addr {
    Ipv4Addr::new(192, 168, 66, 2)
}
struct FakeUbus {
    calls: Vec<String>,
    grant: bool,
    fail_access: bool,
}
impl FakeUbus {
    fn new() -> Self {
        Self {
            calls: vec![],
            grant: true,
            fail_access: false,
        }
    }
}
impl Ubus for FakeUbus {
    fn call(&mut self, _object: &str, method: &str, params: Value) -> Result<Value> {
        self.calls.push(method.to_owned());
        match method {
            "login" => Ok(if params["password"] == "synthetic" {
                json!({"ubus_rpc_session":"a".repeat(32)})
            } else {
                json!({})
            }),
            "access" => {
                if self.fail_access {
                    Err(PanelError::new("SYSTEM_UNAVAILABLE", 503))
                } else {
                    Ok(json!({"access":self.grant}))
                }
            }
            _ => Ok(json!({})),
        }
    }
}
#[test]
fn auth_bound_to_peer_and_deadline_and_revoke() {
    let mut sec = Security::default();
    let mut ubus = FakeUbus::new();
    let login = sec
        .login(&mut ubus, "root", "synthetic", peer(), at(100))
        .unwrap();
    let token = login["token"].as_str().unwrap();
    assert_eq!(ubus.calls, ["login", "access", "destroy"]);
    assert!(sec.authenticate(token, peer(), at(1899)).is_ok());
    assert!(
        sec.authenticate(token, Ipv4Addr::new(192, 168, 66, 3), at(100))
            .is_err()
    );
    assert!(sec.authenticate(token, peer(), at(1900)).is_err());
    let login = sec
        .login(&mut ubus, "root", "synthetic", peer(), at(1901))
        .unwrap();
    let token = login["token"].as_str().unwrap();
    sec.logout(token);
    assert!(sec.authenticate(token, peer(), at(1901)).is_err());
}
#[test]
fn auth_root_grant_destroyed_even_after_access_failure() {
    for fail_access in [false, true] {
        let mut sec = Security::default();
        let mut ubus = FakeUbus::new();
        ubus.grant = false;
        ubus.fail_access = fail_access;
        assert!(
            sec.login(&mut ubus, "root", "synthetic", peer(), at(0))
                .is_err()
        );
        assert_eq!(ubus.calls, ["login", "access", "destroy"]);
    }
}
#[test]
fn rate_limits_all_invalid_credentials_and_recovers() {
    let mut sec = Security::default();
    let mut ubus = FakeUbus::new();
    for _ in 0..5 {
        assert_eq!(
            sec.login(&mut ubus, "guest", "x", peer(), at(0))
                .unwrap_err()
                .code,
            "LOGIN_FAILED"
        );
    }
    assert_eq!(
        sec.login(&mut ubus, "root", "synthetic", peer(), at(299))
            .unwrap_err()
            .code,
        "LOGIN_RATE_LIMIT"
    );
    assert!(ubus.calls.is_empty());
    assert!(
        sec.login(&mut ubus, "root", "synthetic", peer(), at(300))
            .is_ok()
    );
}
#[test]
fn global_login_budget_is_bounded() {
    let mut sec = Security::default();
    let mut ubus = FakeUbus::new();
    for n in 1..=40 {
        assert!(
            sec.login(
                &mut ubus,
                "guest",
                "x",
                Ipv4Addr::new(192, 168, 66, n),
                at(0)
            )
            .is_err()
        );
    }
    assert_eq!(
        sec.login(&mut ubus, "root", "synthetic", peer(), at(1))
            .unwrap_err()
            .code,
        "LOGIN_RATE_LIMIT"
    );
}
#[test]
fn session_and_confirmation_caps() {
    let mut sec = Security::default();
    let mut ubus = FakeUbus::new();
    for _ in 0..16 {
        assert!(
            sec.login(&mut ubus, "root", "synthetic", peer(), at(0))
                .is_ok()
        );
    }
    assert_eq!(
        sec.login(&mut ubus, "root", "synthetic", peer(), at(0))
            .unwrap_err()
            .code,
        "SESSION_LIMIT"
    );
    for _ in 0..32 {
        assert!(sec.prepare("owner", "modem", "AT+CFUN=0", at(0)).is_ok());
    }
    assert_eq!(
        sec.prepare("owner", "modem", "AT+CFUN=0", at(0))
            .unwrap_err()
            .code,
        "AT_RATE_LIMIT"
    );
    assert!(sec.prepare("owner", "modem", "AT+CFUN=0", at(30)).is_ok());
}
#[test]
fn confirmations_once_bound_and_expiring() {
    let mut sec = Security::default();
    for (owner, modem, command, when) in [
        ("other", "m1", "cmd", 0),
        ("owner", "m2", "cmd", 0),
        ("owner", "m1", "other", 0),
        ("owner", "m1", "cmd", 30),
    ] {
        let value = sec.prepare("owner", "m1", "cmd", at(0)).unwrap();
        let nonce = value["confirmation"].as_str().unwrap();
        assert!(sec.consume(nonce, owner, modem, command, at(when)).is_err());
        assert!(sec.consume(nonce, "owner", "m1", "cmd", at(0)).is_err());
    }
    let value = sec.prepare("owner", "m1", "cmd", at(0)).unwrap();
    let nonce = value["confirmation"].as_str().unwrap();
    assert!(sec.consume(nonce, "owner", "m1", "cmd", at(29)).is_ok());
    assert!(sec.consume(nonce, "owner", "m1", "cmd", at(29)).is_err());
}
#[test]
fn logout_revokes_pending_confirmation() {
    let mut sec = Security::default();
    let mut ubus = FakeUbus::new();
    let login = sec
        .login(&mut ubus, "root", "synthetic", peer(), at(0))
        .unwrap();
    let token = login["token"].as_str().unwrap();
    let owner = sec.authenticate(token, peer(), at(0)).unwrap();
    let value = sec.prepare(&owner, "m1", "cmd", at(0)).unwrap();
    sec.logout(token);
    assert!(
        sec.consume(
            value["confirmation"].as_str().unwrap(),
            &owner,
            "m1",
            "cmd",
            at(0)
        )
        .is_err()
    );
}
#[test]
fn at_validation_ascii_single_command_and_exact_queries() {
    for c in [
        "中",
        "A中",
        "AT;AT+CFUN=0",
        "AT\r\nAT+CFUN=0",
        "AT\0",
        "ATé",
    ] {
        assert!(validate_at(c).is_err());
    }
    assert!(validate_at("AT^DSAMBR=1").is_ok());
    assert!(read_only("at^dsambr=1"));
    for c in [
        "AT^DSAMBR=2",
        "AT+CFUN=0",
        "AT+EVIL?",
        " AT+CSQ",
        "AT+CSQ\n",
    ] {
        assert!(!read_only(c));
    }
}
#[test]
fn http_lan_host_origin_boundaries() {
    let lan = boundary::Lan::new(Ipv4Addr::new(192, 168, 66, 1), 24).unwrap();
    let headers = vec![("Host".into(), "192.168.66.1:1117".into())];
    assert!(lan.check(peer(), &headers).is_ok());
    assert!(lan.check(Ipv4Addr::new(192, 168, 67, 2), &headers).is_err());
    for extra in [
        ("Host", "192.168.66.1:1117"),
        ("Origin", "https://evil.invalid"),
        ("Sec-Fetch-Site", "cross-site"),
    ] {
        let mut h = headers.clone();
        h.push((extra.0.into(), extra.1.into()));
        assert!(lan.check(peer(), &h).is_err());
    }
    assert!(boundary::Lan::new(Ipv4Addr::UNSPECIFIED, 24).is_err());
    assert!(boundary::Lan::new(Ipv4Addr::LOCALHOST, 24).is_err());
}
#[test]
fn http_json_body_smuggling_limits() {
    let good = vec![
        ("Content-Length".into(), "2".into()),
        ("Content-Type".into(), "application/json".into()),
    ];
    assert_eq!(boundary::content_length(&good).unwrap(), 2);
    for (key, v) in [
        ("Content-Length", "2"),
        ("Transfer-Encoding", "chunked"),
        ("Content-Type", "application/json"),
    ] {
        let mut h = good.clone();
        h.push((key.into(), v.into()));
        assert!(boundary::content_length(&h).is_err());
    }
    for raw in [
        b"[]".as_slice(),
        b"{\"v\":NaN}",
        b"{\"v\":Infinity}",
        b"{\"v\":1e999}",
        b"\xff\xff",
    ] {
        assert!(boundary::decode_json(raw).is_err());
    }
    assert!(boundary::decode_json(b"{}").is_ok());
    assert!(boundary::decode_json(&vec![b' '; 16385]).is_err());
}
#[test]
fn contract_preserved_and_refresh_not_retry_safe() {
    let contract: Value =
        serde_json::from_str(include_str!("../../docs/api-contract.json")).unwrap();
    assert_eq!(contract["protocol"], wlmb_server::PROTOCOL);
    assert_eq!(contract["port"], wlmb_server::API_PORT);
    let expected = contract["endpoints"]
        .as_object()
        .unwrap()
        .values()
        .filter(|row| row["verb"] == "POST")
        .map(|row| row["path"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(expected, boundary::POST_ROUTES.iter().copied().collect());
    assert!(!boundary::safe_read("refresh"));
    assert!(!boundary::safe_read("host_execute"));
}
#[test]
fn td_ssb_not_extra_cc_or_copied_scc_signal() {
    let raw = "^MONSC: NR,460,01,627264,1,ABC,1E8,ABCD,-81,-5,16\n^HFREQINFO: 0,7,78,630000,3450000,100000,630000,3450000,100000,78,636664,3549960,100000,636664,3549960,100000\n";
    let data = radio::parse("tdtech", raw).unwrap();
    assert_eq!(data["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(data["carriers"][0]["role"], "pcc");
    assert_eq!(data["carriers"][0]["channel"].as_f64(), Some(630000.));
    assert_eq!(
        data["carriers"][0]["servingChannel"].as_f64(),
        Some(627264.)
    );
    assert_eq!(data["carriers"][0]["rsrp"].as_f64(), Some(-81.));
    assert_eq!(data["carriers"][0]["pci"].as_f64(), Some(488.));
    assert_eq!(data["carriers"][0]["roleSource"], "unique-ssb-envelope");
    assert_eq!(data["carriers"][1]["role"], "scc");
    assert!(data["carriers"][1]["rsrp"].is_null());
    assert!(data["carriers"][1]["pci"].is_null());
}
#[test]
fn td_neighbors_hex_pci_and_coherent_fixed_point() {
    let raw = "^MONNC: LTE,1300,100,-85,-9,-62\n^MONNC: NR,633984,1B0,-768,-104,144\n^MONNC: NR,633984,00B,-99,-11,3\n^MONNC: NR,633984,017,-32768,-32768,32767\n^MONNC: NR,633984,018,-80,-5,144\n";
    let data = radio::parse("tdtech", raw).unwrap();
    let rows = data["neighbors"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    let lte = rows.iter().find(|row| row["rat"] == "LTE").unwrap();
    assert_eq!(lte["pci"].as_f64(), Some(256.));
    assert_eq!(lte["rssi"].as_f64(), Some(-62.));
    let scaled = rows.iter().find(|row| row["pci"] == 432.).unwrap();
    assert_eq!(scaled["rsrp"].as_f64(), Some(-96.));
    assert_eq!(scaled["rsrq"].as_f64(), Some(-13.));
    assert_eq!(scaled["sinr"].as_f64(), Some(18.));
    assert_eq!(scaled["measurementEncoding"], "eighth-db");
    let normal = rows.iter().find(|row| row["pci"] == 11.).unwrap();
    assert_eq!(normal["rsrp"].as_f64(), Some(-99.));
    let missing = rows.iter().find(|row| row["pci"] == 23.).unwrap();
    assert!(missing["rsrp"].is_null());
    assert!(missing["sinr"].is_null());
    let mixed = rows.iter().find(|row| row["pci"] == 24.).unwrap();
    assert_eq!(mixed["rsrp"].as_f64(), Some(-80.));
    assert!(mixed["sinr"].is_null());
}
#[test]
fn td_ambiguous_missing_or_outside_envelopes_do_not_guess_primary() {
    for centers in [
        (630000, 630100, 100000),
        (636000, 636664, 100000),
        (630000, 636664, 0),
    ] {
        let raw = format!(
            "^MONSC: NR,460,01,627264,1,ABC,1E8,ABCD,-81,-5,16\n^HFREQINFO: 0,7,78,{},0,{},0,0,0,78,{},0,{},0,0,0\n",
            centers.0, centers.2, centers.1, centers.2
        );
        let data = radio::parse("tdtech", &raw).unwrap();
        assert_eq!(data["carriers"].as_array().unwrap().len(), 2);
        for row in data["carriers"].as_array().unwrap() {
            assert_eq!(row["role"], "unknown");
            assert!(row["rsrp"].is_null());
        }
    }
}
#[test]
fn invalid_values_cannot_be_fabricated() {
    for text in ["NaN", "inf", "-1256", "--", ""] {
        assert_eq!(radio::numeric(text, -156., -28.), None);
    }
    assert!(radio::parse("generic", &"x".repeat(65537)).is_err());
    assert!(radio::parse("unknown", "").is_err());
    assert!(radio::parse("generic", "+CSQ: 99,99").unwrap()["rssi"].is_null());
}
#[test]
fn typed_lock_does_not_accept_bool_injection_bad_band_or_duplicates() {
    let good = json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"scsType":1,"pci":505}]});
    assert_eq!(
        controls::lock_command(&good).unwrap(),
        "AT^NRFREQLOCK=2,0,1,\"78\",\"633984\",\"1\",\"505\""
    );
    for (key, v) in [
        ("scsType", json!(true)),
        ("arfcn", json!(100)),
        ("pci", json!(1008)),
        ("band", json!("78\";AT+CFUN=0")),
    ] {
        let mut value = good.clone();
        value["entries"][0][key] = v;
        assert!(controls::lock_command(&value).is_err());
    }
    let mut duplicate = good.clone();
    duplicate["entries"] = json!([good["entries"][0], good["entries"][0]]);
    assert!(controls::lock_command(&duplicate).is_err());
}
#[test]
fn typed_lock_read_truncation_and_saved_not_activated() {
    let data = controls::parse_lock("^NRFREQLOCK: 2\n0,1\n78,633984,1,505\nOK\n", "NR").unwrap();
    assert_eq!(data["entries"][0]["pci"], 505);
    assert_eq!(
        controls::parse_lock("AT^NRFREQLOCK?\r\n^NRFREQLOCK: 0\r\nOK\r\n", "NR").unwrap()["mode"],
        0
    );
    for raw in [
        "^NRFREQLOCK: 2\n0,2\n78,633984,1,505\nOK",
        "^NRFREQLOCK: 0\nERROR",
        "^NRFREQLOCK: 2\n0,1\n78,633984,9,505\nOK",
        "^NRFREQLOCK: 0\nextra\nOK",
        "^NRFREQLOCK: 0\nOK\n^NRFREQLOCK: 3\n1,1\n78",
        "^NRFREQLOCK: 0\nOK\nERROR",
        "OK\n^NRFREQLOCK: 0\nOK",
        "^NRFREQLOCK: 3\n1,2\n78\n78\nOK",
        "^NRFREQLOCK: 3\n1,+1\n78\nOK",
    ] {
        assert!(controls::parse_lock(raw, "NR").is_err());
    }
}
#[test]
fn typed_write_acknowledgement_is_exact_and_complete() {
    let command = "AT^NRFREQLOCK=0";
    assert!(controls::write_accepted("\r\nOK\r\n", command));
    assert!(controls::write_accepted(
        "AT^NRFREQLOCK=0\r\nOK\r\n",
        command
    ));
    for raw in [
        "OK\nERROR",
        "OK\nOK",
        "OK\nextra",
        "AT^LTEFREQLOCK=0\nOK",
        "OK\0",
        "",
    ] {
        assert!(!controls::write_accepted(raw, command));
    }
}

#[test]
fn telemetry_uses_delta_counters_not_load_or_phy_speed() {
    use wlmb_server::telemetry::{
        NetSample, cpu_percent, cpu_times, memory, rate, station_rates, wan_device,
    };
    assert_eq!(cpu_times("cpu 1 2 3 4 5 6 7 8 100 100\n"), Some((36, 9)));
    assert_eq!(cpu_percent(None, (16, 15)), None);
    assert_eq!(cpu_percent(Some((0, 0)), (16, 15)), Some(6.2));
    assert_eq!(cpu_percent(Some((100, 20)), (90, 30)), None);
    assert_eq!(cpu_percent(Some((100, 20)), (200, 10)), None);
    assert_eq!(cpu_percent(Some((100, 20)), (120, 70)), None);
    let old = NetSample {
        interface: "wwan0",
        mono_seconds: 10.,
        rx: 100,
        tx: 200,
    };
    let current = NetSample {
        interface: "wwan0",
        mono_seconds: 12.,
        rx: 250000100,
        tx: 125000200,
    };
    assert_eq!(
        rate(Some(&old), &current).unwrap()["downloadBytesPerSecond"],
        125000000.
    );
    assert_eq!(
        station_rates((10., 200, 100), (12., 125000200, 250000100)).unwrap()["downloadBytesPerSecond"],
        125000000.
    );
    assert!(
        rate(
            Some(&old),
            &NetSample {
                interface: "wwan0",
                mono_seconds: 12.,
                rx: 0,
                tx: 0
            }
        )
        .is_none()
    );
    assert!(
        rate(
            Some(&old),
            &NetSample {
                interface: "wwan1",
                mono_seconds: 12.,
                rx: 1000,
                tx: 1000
            }
        )
        .is_none()
    );
    assert_eq!(
        memory("MemTotal: 1024 kB\nMemAvailable: 768 kB\n").unwrap()["usedPercent"],
        25.
    );
    let dump = json!({"interface":[{}, {"up":true,"interface":"wan","l3_device":"wwan0","route":[{"target":"0.0.0.0","mask":0,"metric":0}]}]});
    assert_eq!(wan_device(&dump), Some("wwan0".into()));
}

#[test]
fn host_sampling_period_includes_io_without_busy_catchup() {
    use std::time::Duration;
    use wlmb_server::telemetry::sample_delay;
    for (elapsed, expected) in [
        (0, 6000),
        (2000, 4000),
        (5900, 100),
        (6000, 100),
        (10000, 100),
    ] {
        assert_eq!(
            sample_delay(Duration::from_millis(elapsed), Duration::from_secs(6)),
            Duration::from_millis(expected)
        );
    }
}
#[test]
fn ipv6_only_and_split_family_wan_selection_never_double_count() {
    use wlmb_server::telemetry::wan_device;
    let mut dump = serde_json::json!({"interface":[{"interface":"wan6","up":true,"l3_device":"wwan0","route":[{"target":"::","mask":0}]}]});
    assert_eq!(wan_device(&dump), Some("wwan0".into()));
    dump["interface"].as_array_mut().unwrap().push(serde_json::json!({"interface":"wan","up":true,"l3_device":"wwan0","route":[{"target":"0.0.0.0","mask":0}]}));
    assert_eq!(wan_device(&dump), Some("wwan0".into()));
    dump["interface"][1]["l3_device"] = serde_json::json!("eth2");
    assert_eq!(wan_device(&dump), None);
    dump["interface"][0]["route"][0]["table"] = serde_json::json!(100);
    assert_eq!(wan_device(&dump), Some("eth2".into()));
}
