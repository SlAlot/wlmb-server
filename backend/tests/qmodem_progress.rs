#![allow(clippy::unwrap_used)]
// 原生通道分阶段发布回归；模拟慢查询，不连接路由器或打开串口。
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use wlmb_server::{
    Result,
    events::wall_ms,
    sampler::{Sampler, Source},
};

struct Native {
    row: Mutex<Value>,
    gate: Mutex<mpsc::Receiver<()>>,
    calls: AtomicUsize,
    published: AtomicBool,
}
impl Source for Native {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![self.row.lock().unwrap().clone()])
    }
    fn query(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String> {
        panic!("原生读取不应发起额外 AT");
    }
    fn info_snapshot_progress(
        &self,
        _: &Value,
        publish: &mut dyn FnMut(Value) -> Result<()>,
    ) -> Result<Option<Value>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let at = wall_ms();
        let mut value = json!({"serving":[{"rat":"NR","channel":627264.0,"pci":214.0,
            "rsrp":-85.0,"observedAtMs":at,"stale":false}],
            "neighbors":[],"carriers":[],"errors":[],"stale":false,"observedAtMs":at});
        publish(value.clone())?;
        self.published.store(true, Ordering::SeqCst);
        self.gate
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        value["neighbors"] = json!([{"rat":"NR","channel":627264.0,"pci":30.0,"observedAtMs":at}]);
        value["neighborState"] = json!({"status":"reported","observedAtMs":at,"stale":false});
        Ok(Some(value))
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(2);
    while !predicate() {
        assert!(Instant::now() < until, "等待模拟采集超时");
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn primary_is_visible_before_slow_inventory_and_provider_changes_discard_old_completion() {
    for changed in [false, true] {
        let (release, gate) = mpsc::channel();
        let source = Arc::new(Native {
            row: Mutex::new(json!({"id":"m1","model":"rm520f-gl","port":"/dev/ttyUSB2",
                "infoTransport":"qmodem-rpc","manufacturer":"quectel","infoNeighborQuery":true})),
            gate: Mutex::new(gate),
            calls: AtomicUsize::new(0),
            published: AtomicBool::new(false),
        });
        let sampler = Sampler::new(source.clone());
        sampler.discover().unwrap();
        sampler.refresh("m1").unwrap();
        wait(|| source.published.load(Ordering::SeqCst));
        let stage = sampler.status(Some("m1")).unwrap()["radio"].clone();
        assert_eq!(stage["serving"][0]["rsrp"], -85.);
        assert_eq!(stage["pending"], true);
        assert_eq!(stage["stale"], false);
        for _ in 0..30 {
            sampler.status(Some("m1")).unwrap();
            sampler.refresh("m1").unwrap();
        }
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        if changed {
            source.row.lock().unwrap()["manufacturer"] = json!("unknown");
            sampler.discover().unwrap();
            assert!(
                sampler.status(Some("m1")).unwrap()["radio"]["serving"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        release.send(()).unwrap();
        if changed {
            assert!(sampler.stop(Duration::from_secs(1)).unwrap());
        } else {
            wait(|| sampler.status(Some("m1")).unwrap()["radio"]["pending"] != true);
        }
        let final_value = sampler.status(Some("m1")).unwrap()["radio"].clone();
        if changed {
            assert!(final_value["neighbors"].as_array().unwrap().is_empty());
        } else {
            assert_eq!(final_value["neighbors"][0]["pci"], 30.);
            assert_eq!(
                final_value["serving"][0]["observedAtMs"],
                stage["serving"][0]["observedAtMs"]
            );
        }
        assert!(sampler.stop(Duration::from_secs(1)).unwrap());
    }
}
