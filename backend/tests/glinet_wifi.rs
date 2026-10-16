#![cfg(unix)]
#![allow(clippy::unwrap_used)]
// 原厂无线对象缺少运行态的回归；不把接口编号、phy 编号或配置值伪装为实测。
use serde_json::{Value, json};
use std::{sync::Mutex, time::Duration};
use wlmb_server::{Result, lan::LanCache, transport::Bus};
struct Wireless {
    config: Value,
    ssid: &'static str,
    calls: Mutex<Vec<String>>,
}
fn fixture() -> Wireless {
    Wireless {
        config: json!({"values":{
            "wifi0":{".type":"wifi-device","band":"2g","disabled":"0","htmode":"EHT40"},
            "wifi1":{".type":"wifi-device","band":"5g","disabled":"0","htmode":"EHT160"},
            "wifi2":{".type":"wifi-device","band":"6g","disabled":"0","htmode":"EHT160"},
            "wifi2g":{".type":"wifi-iface","device":"wifi0","network":"lan","mode":"ap","encryption":"psk2+ccmp","disabled":"1","ssid":"fixture","key":"NEVER_EXPOSE"},
            "wifi5g":{".type":"wifi-iface","device":"wifi1","network":"lan","mode":"ap","encryption":"psk2+ccmp","disabled":"0","ssid":"fixture","key":"NEVER_EXPOSE"},
            "wifi6g":{".type":"wifi-iface","device":"wifi2","network":"lan","mode":"ap","encryption":"ccmp","disabled":"1","ssid":"fixture","key":"NEVER_EXPOSE"},
            "guest5g":{".type":"wifi-iface","device":"wifi1","network":"guest","mode":"ap","encryption":"psk2+ccmp","disabled":"1","ssid":"guest"}
        }}),
        ssid: "fixture",
        calls: Mutex::new(vec![]),
    }
}
impl Bus for Wireless {
    fn call(&self, object: &str, method: &str, args: Value, _timeout: Duration) -> Result<Value> {
        self.calls.lock().unwrap().push(format!(
            "{object}:{method}:{}",
            args["device"].as_str().unwrap_or("")
        ));
        Ok(match (object, method) {
            ("network.interface.lan", "status") => json!({"up":true,"l3_device":"test-lan"}),
            ("network.wireless", "status") => json!({}),
            ("uci", "get") => self.config.clone(),
            ("iwinfo", "devices") => json!({"devices":["wlan0","wlan1"]}),
            ("iwinfo", "info") if args["device"] == "wlan0" => {
                json!({"phy":"phy0","mode":"Master","htmode":"NOHT","txpower":0})
            }
            ("iwinfo", "info") => {
                json!({"phy":"phy0","mode":"Master","ssid":self.ssid,"frequency":5180,"channel":36,"htmode":"HE160","bitrate":1921500,"htmodes":["HE80","HE160"]})
            }
            ("iwinfo", "assoclist") => {
                json!({"results":[{"mac":"02:00:00:00:00:01","rx":{"bytes":100},"tx":{"bytes":200}}]})
            }
            _ => panic!("unexpected call {object}.{method}"),
        })
    }
}
#[test]
fn iwinfo_fallback_preserves_actual_mode_and_binds_only_unique_lan_profiles() {
    let bus = fixture();
    let value = LanCache::default().sample_with_clock(&bus, || 0.);
    assert!(!value.to_string().contains("NEVER_EXPOSE"));
    let rows = value["wireless"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "wifi5g");
    assert_eq!(rows[0]["band"], "5g");
    assert_eq!(rows[0]["channelWidth"], "HE160");
    assert_eq!(rows[0]["enabled"], true);
    assert_eq!(rows[0]["editable"], false);
    assert_eq!(value["devices"].as_array().unwrap().len(), 1);
    assert!(value["devices"][0]["rates"].is_null());
    assert_eq!(
        bus.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "iwinfo:info:wlan1")
            .count(),
        1,
        "不重复读取同一无线接口"
    );
}
#[test]
fn same_ssid_guest_conflicts_and_unmapped_aps_are_read_only_without_lan_station_claims() {
    for same_ssid_guest in [false, true] {
        let mut bus = fixture();
        if same_ssid_guest {
            bus.config["values"]["guest5g"]["ssid"] = json!("fixture");
        } else {
            bus.ssid = "other-runtime";
        }
        let value = LanCache::default().sample_with_clock(&bus, || 0.);
        assert_eq!(value["wireless"][0]["id"], "iwinfo-wlan1");
        assert_eq!(value["wireless"][0]["lanVerified"], false);
        assert_eq!(value["wireless"][0]["editable"], false);
        assert!(value["devices"].as_array().unwrap().is_empty());
        assert_eq!(value["complete"], false);
        assert!(
            !bus.calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.contains("assoclist"))
        );
    }
}
