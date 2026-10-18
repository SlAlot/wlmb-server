#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use wlmb_server::{
    Result,
    host_controls::{self, HostControls, HostIo},
    security::Security,
    traffic::Date,
};
fn snapshot() -> Value {
    json!({"wireless":{"values":{
    "radio0":{".type":"wifi-device","band":"2g","disabled":"0"},"radio1":{".type":"wifi-device","band":"5g","disabled":"0"},
    "ap0":{".type":"wifi-iface","device":"radio0","mode":"ap","network":["lan"],"ssid":"Panel","key":"private-2g-key","encryption":"psk2","disabled":"0"},
    "ap1":{".type":"wifi-iface","device":"radio1","mode":"ap","network":["lan"],"ssid":"Panel-5G","key":"private-5g-key","encryption":"sae-mixed","disabled":"0"}}},
    "lanDevice":"br-lan","wanDevice":"usb0","nftAvailable":true,"flowOffload":false,"devices":[{"mac":"02:00:00:00:00:01","ip":"192.168.1.2"}]})
}
#[test]
fn disabled_guest_profiles_do_not_hide_primary_aps_and_unknown_runtime_cannot_write() {
    let mut s = snapshot();
    for flag in [json!("1"), json!(true), json!(1), json!("true")] {
        s["wireless"]["values"]["guest"] = json!({".type":"wifi-iface","device":"radio1","mode":"ap","network":"guest","disabled":flag});
        assert_eq!(host_controls::profiles(&s["wireless"]).len(), 2);
    }
    s["wireless"]["values"]["guest"]["disabled"] = json!("0");
    assert_eq!(host_controls::profiles(&s["wireless"]).len(), 1);
    s["wireless"]["values"]["radio2"] =
        json!({".type":"wifi-device","band":"6g","htmode":"EHT160"});
    s["wireless"]["values"]["ap2"] = json!({".type":"wifi-iface","device":"radio2","mode":"ap","network":"lan","ssid":"six","encryption":"ccmp","disabled":"1","key":"NEVER_EXPOSE"});
    let display = host_controls::display_profiles(&s["wireless"]);
    assert_eq!(display.len(), 3);
    assert!(
        !serde_json::to_string(&display)
            .unwrap()
            .contains("NEVER_EXPOSE")
    );
    assert_eq!(
        display.iter().find(|r| r["band"] == "6g").unwrap()["enabled"],
        false
    );
    s["radioCapabilities"]["__runtimeManaged"] = json!(false);
    assert!(host_controls::editable_profiles(&s).is_empty());
    assert!(
        host_controls::wifi_changes(
            &s,
            &json!({"kind":"wifi_edit","band":"2g","changes":{"ssid":"x"}})
        )
        .is_err()
    );
}
#[test]
fn wifi_config_typed_disabled_flags_are_consistent() {
    for flag in [json!(true), json!(1), json!("1"), json!("true")] {
        let mut value = snapshot();
        value["wireless"]["values"]["radio0"]["disabled"] = flag;
        let profiles = host_controls::profiles(&value["wireless"]);
        assert_eq!(
            profiles.iter().find(|p| p["band"] == "2g").unwrap()["enabled"],
            false
        );
    }
    assert!(
        host_controls::profiles(&snapshot()["wireless"])
            .iter()
            .all(|p| p["enabled"] == true)
    );
}
#[test]
fn wifi_options_are_descending_with_auto_channel_first() {
    let mut s = snapshot();
    s["radioCapabilities"] = json!({"radio1":{"saeSupported":true,"info":{"htmodes":["HT20","HT40","VHT80","HE20","HE160","EHT20","EHT80","EHT160"]},"frequencies":{"results":[{"band":5,"channel":36,"restricted":false},{"band":5,"channel":149,"restricted":false},{"band":5,"channel":100,"restricted":true}]}}});
    let rows = host_controls::editable_profiles(&s);
    let p = rows.iter().find(|p| p["band"] == "5g").unwrap();
    let values = |field: &str| {
        p[field]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["value"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(values("modeOptions"), ["EHT", "HE", "VHT", "HT"]);
    assert_eq!(
        values("bandwidthOptions"),
        [
            "EHT160", "EHT80", "EHT20", "HE160", "HE20", "VHT80", "HT40", "HT20"
        ]
    );
    assert_eq!(
        values("securityOptions"),
        ["sae", "sae-mixed", "psk2+ccmp", "psk2"]
    );
    assert_eq!(values("channelOptions"), ["auto", "149", "36"]);
}
struct Fake {
    s: Mutex<Value>,
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    calls: Mutex<Vec<Value>>,
    associated: Mutex<BTreeSet<String>>,
    slow: AtomicBool,
    entered: AtomicBool,
}
impl Fake {
    fn delay(&self) {
        if self.slow.load(Ordering::Acquire) {
            self.entered.store(true, Ordering::Release);
            let until = Instant::now() + Duration::from_secs(3);
            while self.slow.load(Ordering::Acquire) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}
impl HostIo for Fake {
    fn known_device(&self, s: &Value, address: &str) -> Result<bool> {
        Ok(s["devices"]
            .as_array()
            .is_some_and(|a| a.iter().any(|d| d["mac"] == address))
            || self.associated.lock().unwrap().contains(address))
    }
    fn snapshot(&self) -> Result<Value> {
        Ok(self.s.lock().unwrap().clone())
    }
    fn save(&self, n: &str, raw: &[u8]) -> Result<()> {
        self.delay();
        self.files.lock().unwrap().insert(n.into(), raw.to_vec());
        Ok(())
    }
    fn load(&self, n: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.files.lock().unwrap().get(n).cloned())
    }
    fn wifi(&self, _: &Value, changes: &[Value]) -> Result<()> {
        self.calls.lock().unwrap().push(json!({"wifi":changes}));
        let mut s = self.s.lock().unwrap();
        for c in changes {
            for (k, v) in c["changes"].as_object().unwrap() {
                if k == ".radioDisabled" {
                    s["wireless"]["values"][c["profile"]["radio"].as_str().unwrap()]["disabled"] =
                        v.clone();
                } else {
                    s["wireless"]["values"][c["profile"]["id"].as_str().unwrap()][k] = v.clone();
                }
            }
        }
        Ok(())
    }
    fn guard(&self, s: &Value, blocked: &BTreeSet<String>, cutoff: bool) -> Result<()> {
        self.delay();
        host_controls::guard_script(s, blocked, cutoff)?;
        self.calls
            .lock()
            .unwrap()
            .push(json!({"blocked":blocked,"cutoff":cutoff}));
        Ok(())
    }
    fn clear_guard(&self) -> Result<()> {
        self.delay();
        self.calls.lock().unwrap().push(json!({"clear":true}));
        Ok(())
    }
}
fn setup() -> (Arc<Fake>, Arc<HostControls>) {
    let io = Arc::new(Fake {
        s: Mutex::new(snapshot()),
        files: Mutex::new(BTreeMap::new()),
        calls: Mutex::new(vec![]),
        associated: Mutex::new(BTreeSet::new()),
        slow: AtomicBool::new(false),
        entered: AtomicBool::new(false),
    });
    let h = HostControls::new(
        io.clone(),
        Arc::new(Mutex::new(Security::default())),
        Instant::now(),
    )
    .unwrap();
    (io, h)
}
#[test]
fn legacy_traffic_checkpoint_does_not_disable_host_controls() {
    let (io, _) = setup();
    io.files.lock().unwrap().insert(
        "traffic.json".into(),
        serde_json::to_vec(&json!({"format":1,"day":"2026-10-10","month":"2026-10-01",
            "todayBytes":100,"monthBytes":200}))
        .unwrap(),
    );
    let h = HostControls::new(
        io.clone(),
        Arc::new(Mutex::new(Security::default())),
        Instant::now(),
    )
    .unwrap();
    h.sample(
        Some(Date::parse("2026-10-10").unwrap()),
        Some(("usb0", 5000, 6000)),
    )
    .unwrap();
    let public = h.public().unwrap();
    assert_eq!(public["traffic"]["monthBytes"], 200);
    assert_eq!(
        public["wifiControls"]["profiles"].as_array().unwrap().len(),
        2
    );
    assert_eq!(public["deviceBlockSupported"], true);
    assert_eq!(public["trafficLimit"]["enabled"], false);
}
fn cache_during_io(
    io: &Arc<Fake>,
    h: &Arc<HostControls>,
    work: impl FnOnce() -> Result<()> + Send + 'static,
) {
    io.entered.store(false, Ordering::Release);
    io.slow.store(true, Ordering::Release);
    let worker = std::thread::spawn(work);
    let until = Instant::now() + Duration::from_secs(2);
    while !io.entered.load(Ordering::Acquire) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = h.clone();
    let read = std::thread::spawn(move || {
        let _ = tx.send(reader.public());
    });
    let result = rx.recv_timeout(Duration::from_millis(300));
    io.slow.store(false, Ordering::Release);
    worker.join().unwrap().unwrap();
    read.join().unwrap();
    assert!(
        io.entered.load(Ordering::Acquire),
        "test must enter slow native IO"
    );
    assert!(
        result.is_ok(),
        "public cache must not wait for guard or flash IO"
    );
    result.unwrap().unwrap();
}
#[test]
fn cached_status_remains_responsive_during_slow_guards_and_persistence() {
    let (io, h) = setup();
    let sampling = h.clone();
    cache_during_io(&io, &h, move || {
        sampling.sample(Some(Date::parse("2026-10-10").unwrap()), None)
    });
    let op = json!({"kind":"device_block","mac":"02:00:00:00:00:01","blocked":true});
    let prepared = h.prepare(&op, "owner").unwrap();
    let writing = h.clone();
    cache_during_io(&io, &h, move || {
        execute(&writing, &op, &prepared, "owner").map(|_| ())
    });
    assert_eq!(h.public().unwrap()["blockApplyError"], false);
}
#[test]
fn a_station_without_arp_can_be_restricted_but_is_rechecked_before_execute() {
    let (io, h) = setup();
    let address = "02:00:00:00:00:02";
    let op = json!({"kind":"device_block","mac":address,"blocked":true});
    assert!(h.prepare(&op, "owner").is_err());
    io.associated.lock().unwrap().insert(address.into());
    let p = h.prepare(&op, "owner").unwrap();
    io.associated.lock().unwrap().remove(address);
    assert_eq!(
        execute(&h, &op, &p, "owner").unwrap_err().code,
        "DEVICE_NOT_FOUND"
    );
    assert!(io.calls.lock().unwrap().is_empty());
    io.associated.lock().unwrap().insert(address.into());
    let p = h.prepare(&op, "owner").unwrap();
    execute(&h, &op, &p, "owner").unwrap();
    assert!(
        h.public().unwrap()["blocked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == address)
    );
}
fn execute(h: &HostControls, op: &Value, p: &Value, owner: &str) -> Result<Value> {
    h.execute(
        op,
        p["revision"].as_str().unwrap(),
        p["confirmation"].as_str().unwrap(),
        owner,
    )
}
#[test]
fn wifi_only_targets_exclusive_simple_lan_aps_and_never_exposes_keys() {
    let mut s = snapshot();
    assert_eq!(host_controls::profiles(&s["wireless"]).len(), 2);
    let (io, h) = setup();
    h.sample(Some(Date::parse("2026-10-10").unwrap()), None)
        .unwrap();
    assert!(!h.public().unwrap().to_string().contains("private-"));
    assert!(io.files.lock().unwrap().is_empty());
    s["wireless"]["values"]["uplink"] =
        json!({".type":"wifi-iface","device":"radio1","network":"wan","mode":"sta"});
    assert_eq!(host_controls::profiles(&s["wireless"]).len(), 1);
    s["wireless"]["values"]["ap0"]["network"] = json!(["lan", "guest"]);
    assert!(host_controls::profiles(&s["wireless"]).is_empty());
}
#[test]
fn wifi_radio_options_are_driver_bound_and_target_radio_not_ap() {
    let mut s = snapshot();
    s["radioCapabilities"] = json!({"radio1":{"info":{"htmodes":["EHT80","EHT160","UNKNOWN"]},"frequencies":{"results":[{"band":5,"channel":36,"restricted":false},{"band":5,"channel":60,"restricted":true},{"band":2,"channel":6,"restricted":false}]}}});
    let profiles = host_controls::editable_profiles(&s);
    let p = profiles.iter().find(|p| p["band"] == "5g").unwrap();
    assert_eq!(p["channelOptions"].as_array().unwrap().len(), 2);
    assert_eq!(p["bandwidthOptions"].as_array().unwrap().len(), 2);
    assert!(!p.to_string().contains("private-"));
    let change=host_controls::wifi_changes(&s,&json!({"kind":"wifi_edit","band":"5g","changes":{"channel":"36","bandwidth":"EHT80","ssid":"New"}})).unwrap();
    assert_eq!(change["changes"][".radioChannel"], "36");
    assert_eq!(change["changes"][".radioHtmode"], "EHT80");
    for bad in [
        json!({"channel":"60"}),
        json!({"channel":"6"}),
        json!({"bandwidth":"EHT320"}),
        json!({"channel":"36; reboot"}),
    ] {
        assert!(
            host_controls::wifi_changes(&s, &json!({"kind":"wifi_edit","band":"5g","changes":bad}))
                .is_err()
        );
    }
}
#[test]
fn absent_driver_caps_do_not_guess_or_enable_radio_edits() {
    let s = snapshot();
    let p = host_controls::editable_profiles(&s);
    assert_eq!(p[0]["channelSupported"], false);
    assert_eq!(p[0]["bandwidthSupported"], false);
    assert!(
        host_controls::wifi_changes(
            &s,
            &json!({"kind":"wifi_edit","band":"2g","changes":{"bandwidth":"HT40"}})
        )
        .is_err()
    );
    assert!(
        host_controls::wifi_changes(
            &s,
            &json!({"kind":"wifi_edit","band":"2g","changes":{"ssid":"Still editable"}})
        )
        .is_ok()
    );
}
#[test]
fn owner_bound_once_confirmation_and_changed_config_block_wifi_write() {
    let (io, h) = setup();
    let op = json!({"kind":"wifi_edit","band":"5g","changes":{"ssid":"New"}});
    let p = h.prepare(&op, "owner").unwrap();
    assert!(execute(&h, &op, &p, "other").is_err());
    assert!(execute(&h, &op, &p, "owner").is_err());
    let p = h.prepare(&op, "owner").unwrap();
    io.s.lock().unwrap()["wireless"]["values"]["ap1"]["ssid"] = json!("Changed externally");
    assert_eq!(
        execute(&h, &op, &p, "owner").unwrap_err().code,
        "HOST_CHANGED"
    );
    assert!(io.calls.lock().unwrap().is_empty());
}
#[test]
fn device_rate_changes_do_not_invalidate_confirmation_and_unify_restores_exact_credentials() {
    let (io, h) = setup();
    let op = json!({"kind":"wifi_unify","enabled":true});
    let original = io.s.lock().unwrap()["wireless"].clone();
    let p = h.prepare(&op, "owner").unwrap();
    io.s.lock().unwrap()["devices"][0]["rates"] = json!({"downloadBytesPerSecond":100});
    execute(&h, &op, &p, "owner").unwrap();
    assert_eq!(
        io.s.lock().unwrap()["wireless"]["values"]["ap1"]["key"],
        "private-2g-key"
    );
    h.sample(Some(Date::parse("2026-10-10").unwrap()), None)
        .unwrap();
    assert_eq!(
        h.public().unwrap()["wifiControls"]["bandUnifyEnabled"],
        true
    );
    let off = json!({"kind":"wifi_unify","enabled":false});
    let p = h.prepare(&off, "owner").unwrap();
    execute(&h, &off, &p, "owner").unwrap();
    assert_eq!(io.s.lock().unwrap()["wireless"], original);
    assert!(!h.public().unwrap().to_string().contains("private-"));
}
#[test]
fn validation_rejects_unknown_options_multicast_mac_and_firewall_injection() {
    let s = snapshot();
    for changes in [
        json!({"password":"short"}),
        json!({"ssid":""}),
        json!({"channel":36}),
        json!({"enabled":1}),
    ] {
        assert!(
            host_controls::wifi_changes(
                &s,
                &json!({"kind":"wifi_edit","band":"5g","changes":changes})
            )
            .is_err()
        );
    }
    assert!(!host_controls::mac("ff:ff:ff:ff:ff:ff"));
    assert!(!host_controls::mac("02:00:00:00:00:01;flush"));
    let script =
        host_controls::guard_script(&s, &BTreeSet::from(["02:00:00:00:00:01".into()]), true)
            .unwrap();
    assert!(script.contains("hook forward"));
    assert!(!script.contains("hook input"));
    assert!(!script.contains("flush ruleset"));
    let mut bad = s;
    bad["lanDevice"] = json!("br-lan\";flush ruleset");
    assert!(host_controls::guard_script(&bad, &BTreeSet::new(), true).is_err());
}
#[test]
fn guard_disabled_when_accelerated_and_budget_only_counts_observed_delta() {
    let (io, h) = setup();
    io.s.lock().unwrap()["flowOffload"] = json!(true);
    let mut op = json!({"kind":"traffic_limit","enabled":true,"limitBytes":1000,"startDay":1,"threshold":90,"cutoffEnabled":true});
    assert!(h.prepare(&op, "owner").is_err());
    op["cutoffEnabled"] = json!(false);
    let p = h.prepare(&op, "owner").unwrap();
    execute(&h, &op, &p, "owner").unwrap();
    let day = Some(Date::parse("2026-10-10").unwrap());
    h.sample(day, Some(("usb0", 10000, 10000))).unwrap();
    h.sample(day, Some(("usb0", 10500, 10500))).unwrap();
    assert_eq!(
        h.public().unwrap()["deviceBlockReason"],
        "FLOW_OFFLOAD_ENABLED"
    );
    assert_eq!(h.public().unwrap()["traffic"]["monthBytes"], 1000);
    assert_eq!(h.public().unwrap()["trafficLimit"]["warning"], true);
    assert_eq!(
        h.public().unwrap()["trafficLimit"]["cutoffSupported"],
        false
    );
    assert!(!io.calls.lock().unwrap().iter().any(|v| v["cutoff"] == true));
}
#[test]
fn device_block_reasons_match_the_existing_safety_gate_without_changing_configuration() {
    let good = snapshot();
    assert_eq!(host_controls::guard_unavailable_reason(&good), None);
    for (key, value, code) in [
        ("nftAvailable", json!(false), "FIREWALL_UNAVAILABLE"),
        ("flowStateKnown", json!(false), "FLOW_OFFLOAD_UNCONFIRMED"),
        ("flowOffload", json!(true), "FLOW_OFFLOAD_ENABLED"),
        ("flowOffload", Value::Null, "FLOW_OFFLOAD_UNCONFIRMED"),
        ("lanDevice", Value::Null, "LAN_INTERFACE_UNCONFIRMED"),
        ("wanDevice", Value::Null, "WAN_INTERFACE_UNCONFIRMED"),
        ("wanDevice", json!("br-lan"), "FORWARD_PATH_AMBIGUOUS"),
    ] {
        let mut state = good.clone();
        state[key] = value;
        assert!(!host_controls::guard_supported(&state));
        assert_eq!(host_controls::guard_unavailable_reason(&state), Some(code));
        assert!(host_controls::guard_script(&state, &BTreeSet::new(), true).is_err());
    }
}
#[test]
fn blocked_devices_persist_and_can_be_unblocked_after_leaving_lan() {
    let (io, h) = setup();
    let on = json!({"kind":"device_block","mac":"02:00:00:00:00:01","blocked":true});
    let p = h.prepare(&on, "owner").unwrap();
    execute(&h, &on, &p, "owner").unwrap();
    io.s.lock().unwrap()["devices"] = json!([]);
    io.s.lock().unwrap()["flowOffload"] = json!(true);
    io.s.lock().unwrap()["wanDevice"] = Value::Null;
    let off = json!({"kind":"device_block","mac":"02:00:00:00:00:01","blocked":false});
    let p = h.prepare(&off, "owner").unwrap();
    execute(&h, &off, &p, "owner").unwrap();
    assert_eq!(h.public().unwrap()["blocked"], json!([]));
    assert!(execute(&h, &off, &p, "owner").is_err());
}
#[test]
fn sae_rejects_raw_psk_and_disabled_radios_remain_editable() {
    let mut s = snapshot();
    s["wireless"]["values"]["radio1"]["disabled"] = json!("1");
    let on = host_controls::wifi_changes(
        &s,
        &json!({"kind":"wifi_edit","band":"5g","changes":{"enabled":true}}),
    )
    .unwrap();
    assert_eq!(on["changes"][".radioDisabled"], "0");
    let pass = "a".repeat(64);
    assert!(
        host_controls::wifi_changes(
            &s,
            &json!({"kind":"wifi_edit","band":"5g","changes":{"password":pass}})
        )
        .is_err()
    );
    assert!(
        host_controls::wifi_changes(
            &s,
            &json!({"kind":"wifi_edit","band":"2g","changes":{"password":pass}})
        )
        .is_ok()
    );
}

#[test]
fn wifi_generations_security_and_widths_are_validated_together() {
    let mut s = snapshot();
    s["wireless"]["values"]["radio1"]["htmode"] = json!("HE160");
    s["radioCapabilities"]["radio1"] =
        json!({"saeSupported":true,"info":{"htmodes":["HE80","HE160","EHT80","EHT160"]}});
    let p = host_controls::editable_profiles(&s)
        .into_iter()
        .find(|r| r["band"] == "5g")
        .unwrap();
    assert_eq!(p["mode"], "HE");
    assert!(p["modeOptions"].to_string().contains("WiFi 7"));
    assert!(
        p["bandwidthOptions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| !w["label"].as_str().unwrap().contains("HE"))
    );
    let edit = |changes| {
        host_controls::wifi_changes(
            &s,
            &json!({"kind":"wifi_edit","band":"5g","changes":changes}),
        )
    };
    let change = edit(json!({"mode":"EHT","bandwidth":"EHT160","authMode":"sae"})).unwrap();
    assert_eq!(change["changes"][".radioHtmode"], "EHT160");
    assert_eq!(change["changes"]["encryption"], "sae");
    assert!(edit(json!({"mode":"EHT","bandwidth":"HE160"})).is_err());
    assert!(edit(json!({"mode":"EHT","bandwidth":"EHT160","authMode":"psk2"})).is_err());
    assert!(edit(json!({"authMode":"none"})).is_err());
    s["wireless"]["values"]["ap1"]["key"] = json!("a".repeat(64));
    assert_eq!(
        host_controls::wifi_changes(
            &s,
            &json!({"kind":"wifi_edit","band":"5g","changes":{"authMode":"sae"}})
        )
        .unwrap_err()
        .code,
        "WIFI_PASSWORD_REQUIRED"
    );
    assert!(host_controls::wifi_changes(&s,&json!({"kind":"wifi_edit","band":"5g","changes":{"authMode":"sae","password":"new-secure-password"}})).is_ok());
    s["radioCapabilities"]["radio1"]["saeSupported"] = json!(false);
    assert!(
        host_controls::wifi_changes(
            &s,
            &json!({"kind":"wifi_edit","band":"5g","changes":{"authMode":"sae"}})
        )
        .is_err()
    );
}

#[test]
fn band_unification_checks_destination_security_before_any_write() {
    let (io, h) = setup();
    io.s.lock().unwrap()["wireless"]["values"]["radio1"]["htmode"] = json!("EHT160");
    io.s.lock().unwrap()["radioCapabilities"]["radio1"] =
        json!({"saeSupported":true,"info":{"htmodes":["EHT160"]}});
    let op = json!({"kind":"wifi_unify","enabled":true});
    assert_eq!(
        h.prepare(&op, "owner").unwrap_err().code,
        "WIFI7_SECURITY_REQUIRED"
    );
    assert!(io.files.lock().unwrap().is_empty());
    assert!(io.calls.lock().unwrap().is_empty());
}

fn external_unified() -> Value {
    let mut s = snapshot();
    s["wireless"]["values"]["ap1"]["ssid"] = json!("Panel");
    s["wireless"]["values"]["ap1"]["key"] = json!("private-2g-key");
    s["wireless"]["values"]["ap1"]["encryption"] = json!("psk2");
    s
}

#[test]
fn external_unification_is_detected_without_a_plugin_backup() {
    let (io, h) = setup();
    *io.s.lock().unwrap() = external_unified();
    h.sample(None, None).unwrap();
    let public = h.public().unwrap();
    assert_eq!(public["wifiControls"]["bandUnifyEnabled"], true);
    assert_eq!(public["wifiControls"]["bandUnifyStateKnown"], true);
    assert_eq!(public["wifiControls"]["bandUnifyManaged"], false);
    assert!(!public.to_string().contains("private-2g-key"));
    let on = json!({"kind":"wifi_unify","enabled":true});
    let prepared = h.prepare(&on, "owner").unwrap();
    execute(&h, &on, &prepared, "owner").unwrap();
    assert!(
        !io.calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call.get("wifi").is_some()),
        "Already unified must not write WiFi"
    );
    assert!(!io.files.lock().unwrap().contains_key("wifi-backup.json"));
    let off = json!({"kind":"wifi_unify","enabled":false});
    let prepared = h.prepare(&off, "owner").unwrap();
    assert!(prepared["warning"].as_str().unwrap().contains("_5G"));
    execute(&h, &off, &prepared, "owner").unwrap();
    let s = io.s.lock().unwrap();
    assert_eq!(s["wireless"]["values"]["ap1"]["ssid"], "Panel_5G");
    assert_eq!(s["wireless"]["values"]["ap1"]["key"], "private-2g-key");
    assert_eq!(s["wireless"]["values"]["ap1"]["encryption"], "psk2");
}

#[test]
fn unified_state_checks_credentials_enabled_status_and_missing_data() {
    let mut s = external_unified();
    assert_eq!(host_controls::wifi_unified(&s), Some(true));
    for (key, value) in [
        ("key", "different-password"),
        ("encryption", "sae"),
        ("disabled", "1"),
    ] {
        let old = s["wireless"]["values"]["ap1"][key].clone();
        s["wireless"]["values"]["ap1"][key] = json!(value);
        assert_eq!(host_controls::wifi_unified(&s), Some(false));
        s["wireless"]["values"]["ap1"][key] = old;
    }
    s["wireless"]["values"]["ap1"]["key"] = Value::Null;
    assert_eq!(host_controls::wifi_unified(&s), None);
}

#[test]
fn active_mlo_is_not_mistaken_for_exclusive_editable_radios() {
    let mut s = external_unified();
    s["wireless"]["values"]["mlo"] =
        json!({".type":"wifi-iface","device":["radio0","radio1"],"mode":"ap","disabled":"1"});
    assert_eq!(host_controls::wifi_unified(&s), Some(true));
    s["wireless"]["values"]["mlo"]["disabled"] = json!("0");
    assert!(host_controls::editable_profiles(&s).is_empty());
    assert_eq!(host_controls::wifi_unified(&s), None);
}

fn open_wireless() -> Value {
    let mut s = snapshot();
    for (radio, mode) in [("radio0", "EHT40"), ("radio1", "EHT160")] {
        s["wireless"]["values"][radio]["type"] = json!("mtwifi");
        s["wireless"]["values"][radio]["htmode"] = json!(mode);
    }
    for ap in ["ap0", "ap1"] {
        s["wireless"]["values"][ap]["encryption"] = json!("none");
        s["wireless"]["values"][ap]
            .as_object_mut()
            .unwrap()
            .remove("key");
    }
    s["radioCapabilities"] = json!({"radio1":{"saeSupported":false,"info":{"htmodes":["HE160","EHT160","EHT80"]},"frequencies":{"results":[{"band":5,"channel":149,"restricted":false}]}}});
    s
}

#[test]
fn system_open_networks_keep_readable_profiles_and_verified_edit_options() {
    let s = open_wireless();
    let p = host_controls::editable_profiles(&s);
    assert_eq!(p.len(), 2);
    let five = p.iter().find(|p| p["band"] == "5g").unwrap();
    assert_eq!(five["enabled"], true);
    assert_eq!(five["authMode"], "none");
    assert_eq!(five["modeOptions"][0]["label"], "WiFi 7");
    assert!(
        five["securityOptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["value"] == "none" && o["label"] == "开放网络（无密码）")
    );
    assert!(
        !five["securityOptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["value"] == "sae")
    );
    assert!(host_controls::wifi_changes(&s, &json!({"kind":"wifi_edit","band":"5g","changes":{"ssid":"New","channel":"149","bandwidth":"EHT80"}})).is_ok());
    assert_eq!(
        host_controls::wifi_changes(
            &s,
            &json!({"kind":"wifi_edit","band":"5g","changes":{"password":"private-new-pass"}})
        )
        .unwrap_err()
        .code,
        "WIFI_PASSWORD_REQUIRES_SECURITY"
    );
    assert!(host_controls::wifi_changes(&s, &json!({"kind":"wifi_edit","band":"5g","changes":{"mode":"HE","bandwidth":"HE160","authMode":"psk2","password":"private-new-pass"}})).is_ok());
}

#[test]
fn editing_existing_driver_reported_wifi7_width_does_not_force_a_security_change() {
    let mut s = open_wireless();
    s["wireless"]["values"]["ap1"]["encryption"] = json!("psk2");
    s["wireless"]["values"]["ap1"]["key"] = json!("private-existing-key");
    let changed = host_controls::wifi_changes(
        &s,
        &json!({"kind":"wifi_edit","band":"5g","changes":{"bandwidth":"EHT80"}}),
    )
    .unwrap();
    assert_eq!(changed["changes"], json!({".radioHtmode":"EHT80"}));
}

#[test]
fn open_network_unification_does_not_require_or_invent_a_password_and_restores_exactly() {
    let (io, h) = setup();
    *io.s.lock().unwrap() = open_wireless();
    h.sample(None, None).unwrap();
    assert_eq!(
        h.public().unwrap()["wifiControls"]["bandUnifyStateKnown"],
        true
    );
    assert_eq!(
        h.public().unwrap()["wifiControls"]["bandUnifySupported"],
        true
    );
    let before = io.s.lock().unwrap()["wireless"].clone();
    let on = json!({"kind":"wifi_unify","enabled":true});
    let prepared = h.prepare(&on, "owner").unwrap();
    execute(&h, &on, &prepared, "owner").unwrap();
    assert_eq!(
        host_controls::wifi_unified(&io.s.lock().unwrap()),
        Some(true)
    );
    assert!(
        io.s.lock().unwrap()["wireless"]["values"]["ap1"]
            .get("key")
            .is_none()
    );
    h.sample(None, None).unwrap();
    let off = json!({"kind":"wifi_unify","enabled":false});
    let prepared = h.prepare(&off, "owner").unwrap();
    execute(&h, &off, &prepared, "owner").unwrap();
    assert_eq!(io.s.lock().unwrap()["wireless"], before);
}

#[test]
fn mixed_open_and_encrypted_unification_never_silently_downgrades_security() {
    let (io, h) = setup();
    let mut s = open_wireless();
    s["wireless"]["values"]["ap1"]["encryption"] = json!("psk2");
    s["wireless"]["values"]["ap1"]["key"] = json!("private-existing-key");
    *io.s.lock().unwrap() = s;
    h.sample(None, None).unwrap();
    let before_calls = io.calls.lock().unwrap().clone();
    let before_files = io.files.lock().unwrap().clone();
    assert_eq!(
        h.public().unwrap()["wifiControls"]["bandUnifySupported"],
        false
    );
    assert_eq!(
        h.prepare(&json!({"kind":"wifi_unify","enabled":true}), "owner")
            .unwrap_err()
            .code,
        "WIFI_UNIFY_SECURITY_REQUIRED"
    );
    assert_eq!(*io.calls.lock().unwrap(), before_calls);
    assert_eq!(*io.files.lock().unwrap(), before_files);
}
