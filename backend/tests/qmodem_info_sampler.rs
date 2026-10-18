#![allow(clippy::unwrap_used)]
// 原生信息采集与缓存回归，不访问路由器，也不使用真实串口。
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use wlmb_server::{
    PanelError, Result,
    events::wall_ms,
    qmodem_info,
    sampler::{Sampler, Source},
};

struct Native {
    row: Mutex<Value>,
    calls: AtomicUsize,
    fail: bool,
}
impl Source for Native {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![self.row.lock().unwrap().clone()])
    }
    fn query(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String> {
        panic!("原生信息路径不能退回 AT 查询")
    }
    fn info_snapshot(&self, row: &Value) -> Result<Option<Value>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(PanelError::new("QMODEM_INFO_EMPTY", 503));
        }
        let mut value = qmodem_info::parse(
            &json!({"modem_info":[{"key":"network_mode","value":"NR5G-SA Mode"},
            {"key":"ARFCN","value":"627264"},{"key":"Physical Cell ID","value":"214"},
            {"key":"RSRP","value":"-85"}]}),
            None,
            wall_ms(),
        )?;
        value["modem"] = row.clone();
        Ok(Some(value))
    }
}
fn source(fail: bool) -> Arc<Native> {
    Arc::new(Native {
        row: Mutex::new(json!({"id":"2_1","port":"/dev/ttyUSB2","model":"rm520f-gl",
        "shared":false,"infoTransport":"qmodem-rpc","instance":"test-one"})),
        calls: AtomicUsize::new(0),
        fail,
    })
}
fn settled(panel: &Sampler) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while panel.status(None).unwrap()["radio"]["pending"] == true {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn auto_sampling_without_open_daemon_deduplicates_refresh_and_invalidates_transport_changes() {
    let source = source(false);
    let panel = Sampler::new(source.clone());
    panel.tick().unwrap();
    settled(&panel);
    assert_eq!(
        panel.status(None).unwrap()["radio"]["serving"][0]["rsrp"],
        -85.
    );
    for _ in 0..100 {
        panel.status(None).unwrap();
        assert_eq!(panel.refresh("2_1").unwrap()["accepted"], false);
    }
    panel.tick().unwrap();
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    {
        let mut row = source.row.lock().unwrap();
        row.as_object_mut().unwrap().remove("infoTransport");
    }
    panel.discover().unwrap();
    assert!(
        panel.status(None).unwrap()["radio"]["serving"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(panel.stop(Duration::from_secs(1)).unwrap());
}
#[test]
fn native_failure_is_visible_in_diagnostics_and_does_not_busy_loop() {
    let source = source(true);
    let panel = Sampler::new(source.clone());
    panel.tick().unwrap();
    settled(&panel);
    let value = panel.status(None).unwrap();
    assert_eq!(value["radio"]["errors"][0]["code"], "QMODEM_INFO_EMPTY");
    assert_eq!(value["radio"]["stale"], true);
    assert_eq!(panel.events.recent()[0]["code"], "QMODEM_INFO_EMPTY");
    for _ in 0..10 {
        panel.tick().unwrap();
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(panel.stop(Duration::from_secs(1)).unwrap());
}
