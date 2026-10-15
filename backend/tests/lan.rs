#![cfg(unix)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
#![allow(clippy::unwrap_used)]
use serde_json::{Value, json};
use std::{sync::Mutex, time::Duration};
use wlmb_server::{
    Result,
    lan::{LanCache, age_snapshot, peers},
    transport::Bus,
};

struct Fake(Mutex<u64>);
#[test]
fn wifi_runtime_unknown_is_not_disabled() {
    assert_eq!(wlmb_server::lan::ap_state(&json!({}), &json!({})), None);
    for up in [json!(true), json!(1), json!("1")] {
        assert_eq!(
            wlmb_server::lan::ap_state(&json!({"up":up}), &json!({})),
            Some(true)
        );
    }
    for flag in [json!(true), json!(1), json!("1"), json!("true")] {
        assert_eq!(
            wlmb_server::lan::ap_state(&json!({"up":true}), &json!({"disabled":flag})),
            Some(false)
        );
    }
}
impl Bus for Fake {
    fn call(&self, object: &str, method: &str, _: Value, _: Duration) -> Result<Value> {
        let bytes = *self.0.lock().unwrap();
        Ok(match (object, method) {
            ("network.interface.lan", "status") => json!({"up":true,"l3_device":"test-lan"}),
            ("network.wireless", "status") => {
                json!({"radio0":{"up":true,"interfaces":[{"section":"main","ifname":"test-wlan","config":{"network":["lan"],"mode":"ap","ssid":"test","key":"NEVER_EXPOSE"}}]}})
            }
            ("iwinfo", "info") => json!({"frequency":5180,"channel":36,"htmode":"HE80"}),
            ("iwinfo", "assoclist") => {
                json!({"results":[{"mac":"02:00:00:00:00:01","rx":{"bytes":bytes,"rate":1000000},"tx":{"bytes":bytes*2}},{"mac":"02:00:00:00:00:02","rx":{"rate":1000000},"tx":{"rate":1000000}}]})
            }
            _ => panic!("Unexpected bus read"),
        })
    }
}
#[test]
fn peers_are_observed_ipv4_not_lease_history_or_wan() {
    let arp = "IP address HW type Flags HW address Mask Device\n192.168.1.2 0x1 0x2 02:00:00:00:00:01 * br-lan\n192.168.1.3 0x1 0x0 02:00:00:00:00:02 * br-lan\n10.0.0.1 0x1 0x2 02:00:00:00:00:03 * wan\n";
    let list = peers(
        arp,
        "0 02:00:00:00:00:01 192.168.1.2 test *\n0 02:00:00:00:00:04 192.168.1.4 offline *",
        &["br-lan".into()],
    );
    assert_eq!(list.len(), 1);
    assert_eq!(list["02:00:00:00:00:01"]["name"], "test");
}
#[test]
fn wireless_is_cached_without_keys_or_fake_phy_speed() {
    let bus = Fake(Mutex::new(100));
    let mut cache = LanCache::default();
    let first = cache.sample_with_clock(&bus, || 0.);
    assert!(!first.to_string().contains("NEVER_EXPOSE"));
    assert_eq!(first["wireless"][0]["band"], "5g");
    assert!(first["devices"][0]["rates"].is_null());
    *bus.0.lock().unwrap() = 300;
    let second = cache.sample_with_clock(&bus, || 2.);
    assert_eq!(
        second["devices"][0]["rates"]["uploadBytesPerSecond"].as_f64(),
        Some(100.)
    );
    assert_eq!(
        second["devices"][0]["rates"]["downloadBytesPerSecond"].as_f64(),
        Some(200.)
    );
    assert!(second["devices"][1]["rates"].is_null());
    *bus.0.lock().unwrap() = 1;
    assert!(cache.sample_with_clock(&bus, || 4.)["devices"][0]["rates"].is_null());
}

struct RadioBus {
    wireless: Value,
    stations: Value,
    calls: Mutex<Vec<Value>>,
}
impl Bus for RadioBus {
    fn call(&self, object: &str, method: &str, params: Value, _: Duration) -> Result<Value> {
        match (object, method) {
            ("network.interface.lan", "status") => Ok(json!({"up":false,"l3_device":"test-lan"})),
            ("network.wireless", "status") => Ok(self.wireless.clone()),
            ("iwinfo", _) => {
                self.calls.lock().unwrap().push(params);
                if method == "assoclist" {
                    Ok(self.stations.clone())
                } else {
                    Ok(json!({"frequency":5180}))
                }
            }
            _ => panic!("Unexpected bus read"),
        }
    }
}
fn radio_bus(up: bool, disabled: Value) -> RadioBus {
    RadioBus {
        wireless: json!({"radio0":{"up":up,"interfaces":[{"section":"main","ifname":"test-wlan","config":{"network":"lan","mode":"ap","ssid":"test","disabled":disabled}}]}}),
        stations: json!({"results":[{"mac":"02:00:00:00:00:01","rx":{"bytes":100},"tx":{"bytes":200}}]}),
        calls: Mutex::new(vec![]),
    }
}
#[test]
fn down_disabled_and_non_lan_interfaces_do_not_report_ghost_stations() {
    for (up, disabled) in [
        (false, json!(false)),
        (true, json!(true)),
        (true, json!(1)),
        (true, json!("1")),
    ] {
        let bus = radio_bus(up, disabled);
        let value = LanCache::default().sample_with_clock(&bus, || 0.);
        assert!(bus.calls.lock().unwrap().is_empty());
        assert_eq!(value["wireless"][0]["enabled"], false);
        assert_eq!(value["devices"], json!([]));
        assert_eq!(value["deviceListSupported"], false);
    }
    for config in [
        json!({"mode":"ap","network":"guest"}),
        json!({"mode":"sta","network":"lan"}),
    ] {
        let mut bus = radio_bus(true, json!(false));
        bus.wireless["radio0"]["interfaces"][0]["config"] = config;
        let value = LanCache::default().sample_with_clock(&bus, || 0.);
        assert!(bus.calls.lock().unwrap().is_empty());
        assert_eq!(value["devices"], json!([]));
    }
}
#[test]
fn malformed_association_is_not_a_successful_empty_device_list() {
    let mut bus = radio_bus(true, json!(false));
    bus.stations = json!({"error":"SYNTHETIC_DRIVER_ERROR"});
    let value = LanCache::default().sample_with_clock(&bus, || 0.);
    assert_eq!(value["complete"], false);
    assert_eq!(value["deviceListSupported"], false);
    bus.stations = json!({"results":[{"mac":"00:00:00:00:00:00"},{"mac":"ff:ff:ff:ff:ff:ff"},{"mac":"01:00:5e:00:00:01"}]});
    let value = LanCache::default().sample_with_clock(&bus, || 0.);
    assert_eq!(value["devices"], json!([]));
    assert_eq!(value["deviceListSupported"], true);
    assert_eq!(value["complete"], true);
}
#[test]
fn station_time_is_taken_after_query_and_stale_cache_is_not_rejuvenated() {
    let bus = Fake(Mutex::new(100));
    let mut cache = LanCache::default();
    cache.sample_with_clock(&bus, || {
        assert_eq!(*bus.0.lock().unwrap(), 100);
        1.
    });
    *bus.0.lock().unwrap() = 300;
    let second = cache.sample_with_clock(&bus, || 5.);
    assert_eq!(second["devices"][0]["rates"]["uploadBytesPerSecond"], 50.);
    assert_eq!(
        second["devices"][0]["rates"]["downloadBytesPerSecond"],
        100.
    );
    for (now, stale) in [(30000, false), (31000, false), (31001, true), (999, true)] {
        let mut value = json!({"observedAtMs":1000,"stale":false,"devices":[]});
        age_snapshot(&mut value, now);
        assert_eq!(value["stale"], stale);
        assert_eq!(value["observedAtMs"], 1000);
    }
    let mut absent = json!({});
    age_snapshot(&mut absent, 0);
    assert_eq!(absent["stale"], true);
}
