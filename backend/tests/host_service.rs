#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use wlmb_server::{
    PanelError, Result,
    host_controls::HostIo,
    sampler::{Sampler, Source},
    service::NativeService,
    transport::Bus,
};
struct Empty;
struct Inventory(Vec<Value>);
impl Source for Inventory {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(self.0.clone())
    }
    fn query(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String> {
        panic!("Status and capability checks must never query a modem")
    }
}

#[test]
fn both_transports_gate_radio_features_on_ready_channel_and_exact_parser() {
    for model in [
        "mt5700m-cn",
        "rm520n-gl",
        "fm350-gl",
        "fn990a40",
        "rg801",
        "em9190",
    ] {
        for (transport, ready) in [("dedicated", false), ("dedicated", true), ("shared", true)] {
            let source = Arc::new(Inventory(vec![
                json!({"id":"m1","model":model,"port":"/dev/ttyUSB1","shared":transport=="shared","transport":transport,"identityVerified":ready,"channelReady":ready}),
            ]));
            let sampler = Sampler::new(source.clone());
            sampler.discover().unwrap();
            let s = NativeService::integrated(Arc::new(Empty), sampler, source, None, None, None)
                .unwrap();
            let status = s.local("status", json!({"modem":"m1"})).unwrap();
            for field in ["signal", "neighbors", "carrierAggregation"] {
                assert_eq!(
                    status["capabilities"][field],
                    ready && wlmb_server::catalog::radio_capabilities(model)[field] == true,
                    "{model}/{transport}/{field}"
                );
            }
            assert_eq!(status["capabilities"]["at"], ready);
            assert_eq!(
                status["capabilities"]["lockBand"],
                ready && wlmb_server::lock_protocol::band_supported(model)
            );
            assert_eq!(
                status["capabilities"]["lockCell"],
                ready && wlmb_server::lock_protocol::cell_supported(model)
            );
        }
    }
}
impl Source for Empty {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![])
    }
    fn query(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String> {
        Err(PanelError::new("MODEM_NOT_READY", 409))
    }
}
impl Bus for Empty {
    fn call(&self, _: &str, _: &str, _: Value, _: Duration) -> Result<Value> {
        Err(PanelError::new("SYSTEM_UNAVAILABLE", 503))
    }
}
struct Io {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    reads: AtomicUsize,
    broken: bool,
}
impl HostIo for Io {
    fn snapshot(&self) -> Result<Value> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"wireless":{"values":{
            "radio0":{".type":"wifi-device","band":"2g","disabled":"1"},
            "ap0":{".type":"wifi-iface","device":"radio0","mode":"ap","network":"lan","ssid":"Disabled AP","key":"synthetic-password","encryption":"psk2","disabled":"1"}}},
            "lanDevice":"br-lan","wanDevice":null,"nftAvailable":false,"flowOffload":true,"devices":[]}))
    }
    fn load(&self, n: &str) -> Result<Option<Vec<u8>>> {
        if self.broken {
            return Err(PanelError::new("PRIVATE_STORE_INVALID", 503));
        }
        Ok(self.files.lock().unwrap().get(n).cloned())
    }
    fn save(&self, n: &str, b: &[u8]) -> Result<()> {
        self.files.lock().unwrap().insert(n.into(), b.to_vec());
        Ok(())
    }
    fn wifi(&self, _: &Value, _: &[Value]) -> Result<()> {
        panic!("No WiFi mutation authorized in this service test")
    }
    fn guard(&self, _: &Value, _: &BTreeSet<String>, _: bool) -> Result<()> {
        panic!("No forwarding guard authorized in this test")
    }
    fn clear_guard(&self) -> Result<()> {
        Ok(())
    }
}
fn setup(broken: bool) -> (Arc<Io>, Arc<NativeService>) {
    let io = Arc::new(Io {
        files: Mutex::new(BTreeMap::new()),
        reads: AtomicUsize::new(0),
        broken,
    });
    let source = Arc::new(Empty);
    let s = NativeService::integrated(
        Arc::new(Empty),
        Sampler::new(source.clone()),
        source,
        None,
        None,
        Some(io.clone()),
    )
    .unwrap();
    (io, s)
}
fn receipt(s: &NativeService, job: &Value, owner: char) -> Value {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let v = s
            .local(
                "control_result",
                json!({"job":job["job"],"ubus_rpc_session":owner.to_string().repeat(32)}),
            )
            .unwrap();
        if v["state"] != "pending" {
            return v;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn limit_receipts_are_owner_bound_and_status_reads_only_cache() {
    let (io, s) = setup(false);
    let op = json!({"kind":"traffic_limit","enabled":true,"limitBytes":10737418240u64,"startDay":15,"threshold":90,"cutoffEnabled":false});
    let job = s
        .local(
            "host_prepare",
            json!({"operation":op.to_string(),"ubus_rpc_session":"a".repeat(32)}),
        )
        .unwrap();
    assert!(
        s.local(
            "control_result",
            json!({"job":job["job"],"ubus_rpc_session":"b".repeat(32)})
        )
        .is_err()
    );
    let prepared = receipt(&s, &job, 'a');
    assert_eq!(prepared["state"], "done");
    let job=s.local("host_execute",json!({"operation":op,"revision":prepared["result"]["revision"],"confirmation":prepared["result"]["confirmation"],"ubus_rpc_session":"a".repeat(32)})).unwrap();
    assert_eq!(receipt(&s, &job, 'a')["state"], "done");
    assert!(io.files.lock().unwrap().contains_key("traffic.json"));
    let reads = io.reads.load(Ordering::SeqCst);
    for _ in 0..4 {
        let status = s.local("status", json!({})).unwrap();
        assert_eq!(status["system"]["trafficLimit"]["startDay"], 15);
        assert_eq!(status["capabilities"]["at"], false);
        for feature in ["signal", "neighbors", "carrierAggregation"] {
            assert_eq!(status["capabilities"][feature], false);
        }
        assert!(!status.to_string().contains("synthetic-password"));
    }
    assert_eq!(io.reads.load(Ordering::SeqCst), reads);
}
#[test]
fn disabled_configured_ap_remains_visible_without_claiming_live_devices() {
    let (_, s) = setup(false);
    let stop = Arc::new(AtomicBool::new(false));
    let worker = s.sample_host(stop.clone());
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let v = s.local("status", json!({})).unwrap();
        if v["system"]["lan"]["wireless"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            assert_eq!(v["system"]["lan"]["wireless"][0]["enabled"], false);
            assert_eq!(v["system"]["lan"]["wireless"][0]["ssid"], "Disabled AP");
            assert_eq!(v["capabilities"]["wifi"], true);
            assert_ne!(v["capabilities"]["deviceList"], true);
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    stop.store(true, Ordering::Release);
    worker.join().unwrap();
}
#[test]
fn optional_private_state_failure_does_not_disable_core_service() {
    let (_, s) = setup(true);
    assert_eq!(s.local("ping", json!({})).unwrap()["backend"], "rust");
    assert_eq!(
        s.local("status", json!({})).unwrap()["capabilities"]["trafficLimit"],
        false
    );
    let diag = s.local("diagnostics", json!({})).unwrap().to_string();
    assert!(diag.contains("PRIVATE_STORE_INVALID"));
    assert!(!diag.contains("synthetic-password"));
}
