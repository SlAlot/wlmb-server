#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use wlmb_server::lan_ports::{PortCache, PortSample, collect, forwarding};
const A: &str = "02:00:00:00:00:01";
const B: &str = "02:00:00:00:00:02";
fn record(address: &str, port: u16, local: bool) -> Vec<u8> {
    let mut row = vec![0; 16];
    for (i, part) in address.split(':').enumerate() {
        row[i] = u8::from_str_radix(part, 16).unwrap();
    }
    row[6] = port as u8;
    row[12] = (port >> 8) as u8;
    row[7] = u8::from(local);
    row
}
fn sample(peers: &[&str], index: u64, rx: u64, tx: u64) -> PortSample {
    PortSample {
        interface: "eth1".into(),
        index,
        rx,
        tx,
        peers: peers.iter().map(|v| (*v).into()).collect(),
    }
}
fn devices() -> BTreeMap<String, Value> {
    [
        (A.into(), json!({"type":"设备","rates":null})),
        (B.into(), json!({"type":"设备","rates":null})),
    ]
    .into()
}
#[test]
fn fdb_abi_high_ports_duplicates_local_multicast_and_bounds() {
    let mut raw = record(A, 257, false);
    raw.extend(record(A, 257, false));
    raw.extend(record(B, 1, true));
    raw.extend(record("01:00:5e:00:00:01", 1, false));
    raw.extend(record("00:00:00:00:00:00", 1, false));
    assert_eq!(
        forwarding(&raw).unwrap(),
        [(257, [A.to_owned()].into())].into()
    );
    assert!(forwarding(&raw[..raw.len() - 1]).is_none());
    assert!(forwarding(&vec![0; 65552]).is_none());
    assert!(forwarding(&[]).unwrap().is_empty());
}
#[test]
fn actual_counter_direction_not_negotiated_link_speed() {
    let mut cache = PortCache::default();
    let mut peers = devices();
    assert!(cache.apply(&mut peers, &[sample(&[A], 1, 100, 200)], 1.)[0]["rates"].is_null());
    let rows = cache.apply(&mut peers, &[sample(&[A], 1, 300, 800)], 3.);
    assert_eq!(rows[0]["rates"]["uploadBytesPerSecond"], 100.);
    assert_eq!(peers[A]["rates"]["downloadBytesPerSecond"], 300.);
    assert_eq!(peers[A]["rateScope"], "port");
    assert_eq!(peers[A]["rateInterface"], "eth1");
}
#[test]
fn shared_ports_never_attribute_the_whole_port_to_each_peer() {
    let mut cache = PortCache::default();
    let mut peers = devices();
    cache.apply(&mut peers, &[sample(&[A, B], 1, 100, 200)], 0.);
    let rows = cache.apply(&mut peers, &[sample(&[A, B], 1, 300, 400)], 2.);
    assert_eq!(rows[0]["peerCount"], 2);
    assert_eq!(rows[0]["rates"]["downloadBytesPerSecond"], 100.);
    assert!(peers[A]["rates"].is_null() && peers[B]["rates"].is_null());
}
#[test]
fn reused_device_cache_drops_old_port_attribution_when_it_becomes_shared() {
    let mut cache = PortCache::default();
    let mut peers = devices();
    cache.apply(&mut peers, &[sample(&[A], 1, 100, 100)], 0.);
    cache.apply(&mut peers, &[sample(&[A], 1, 300, 300)], 2.);
    assert!(!peers[A]["rates"].is_null());
    cache.apply(&mut peers, &[sample(&[A, B], 1, 500, 500)], 4.);
    assert!(peers[A]["rates"].is_null());
    assert!(peers[A]["rateScope"].is_null());
}
#[test]
fn owner_interface_reset_missing_stale_and_wifi_are_not_false_samples() {
    let mut cache = PortCache::default();
    let mut peers = devices();
    cache.apply(&mut peers, &[sample(&[A], 1, 100, 100)], 0.);
    cache.apply(&mut peers, &[sample(&[B], 1, 300, 300)], 2.);
    assert!(peers[B]["rates"].is_null());
    assert!(cache.apply(&mut peers, &[sample(&[B], 2, 400, 400)], 4.)[0]["rates"].is_null());
    assert!(cache.apply(&mut peers, &[sample(&[B], 2, 1, 1)], 6.)[0]["rates"].is_null());
    assert!(cache.apply(&mut peers, &[sample(&[B], 2, 100, 100)], 60.)[0]["rates"].is_null());
    cache.apply(&mut peers, &[], 61.);
    assert!(cache.apply(&mut peers, &[sample(&[B], 2, 200, 200)], 62.)[0]["rates"].is_null());
    peers.get_mut(B).unwrap()["type"] = json!("WiFi");
    peers.get_mut(B).unwrap()["rates"] = json!({"downloadBytesPerSecond":123.});
    cache.apply(&mut peers, &[sample(&[B], 2, 300, 300)], 64.);
    assert_eq!(peers[B]["rates"]["downloadBytesPerSecond"], 123.);
}
#[test]
fn duplicate_cross_port_mac_does_not_pick_a_client_port() {
    let mut cache = PortCache::default();
    let mut peers = devices();
    let one = sample(&[A], 1, 100, 100);
    let mut two = one.clone();
    two.interface = "eth3".into();
    cache.apply(&mut peers, &[one.clone(), two.clone()], 0.);
    cache.apply(&mut peers, &[one, two], 2.);
    assert!(peers[A]["rateScope"].is_null());
}
#[test]
fn physical_bridge_ports_only_and_missing_fdb_fails_closed() {
    let root = std::env::temp_dir().join(format!(
        "wlmb-port-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("br-lan/brif/eth1")).unwrap();
    std::fs::create_dir_all(root.join("eth1/device")).unwrap();
    std::fs::create_dir_all(root.join("eth1/statistics")).unwrap();
    for (p, v) in [
        ("br-lan/brif/eth1/port_no", "0x1"),
        ("br-lan/brif/eth1/state", "3"),
        ("eth1/type", "1"),
        ("eth1/carrier", "1"),
        ("eth1/ifindex", "3"),
        ("eth1/statistics/rx_bytes", "100"),
        ("eth1/statistics/tx_bytes", "200"),
    ] {
        std::fs::write(root.join(p), v).unwrap();
    }
    std::fs::write(root.join("br-lan/brforward"), record(A, 1, false)).unwrap();
    assert_eq!(collect(&root, "br-lan", &BTreeSet::new()).len(), 1);
    assert!(collect(&root, "../br-lan", &BTreeSet::new()).is_empty());
    assert!(collect(&root, "br-lan", &["eth1".into()].into()).is_empty());
    std::fs::create_dir(root.join("eth1/phy80211")).unwrap();
    assert!(collect(&root, "br-lan", &BTreeSet::new()).is_empty());
    std::fs::remove_dir(root.join("eth1/phy80211")).unwrap();
    std::fs::write(root.join("eth1/carrier"), "0").unwrap();
    assert!(collect(&root, "br-lan", &BTreeSet::new()).is_empty());
    std::fs::write(root.join("eth1/carrier"), "1").unwrap();
    std::fs::remove_file(root.join("br-lan/brforward")).unwrap();
    assert!(collect(&root, "br-lan", &BTreeSet::new()).is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
