#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::Instant,
};
use wlmb_server::{
    Result, control_runtime::ModemControls, radio_controls::RadioControls, sampler::Source,
    security::Security,
};
struct Device {
    row: Mutex<Value>,
    calls: Mutex<Vec<String>>,
}
impl Source for Device {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![self.row.lock().unwrap().clone()])
    }
    fn query(&self, _: &str, c: &str, _: &str, _: &str) -> Result<String> {
        self.calls.lock().unwrap().push(c.into());
        Ok("OK\r\n".into())
    }
}
fn setup() -> (Arc<Device>, Arc<ModemControls>) {
    let d = Arc::new(Device {
        row: Mutex::new(
            json!({"id":"m1","port":"/dev/ttyUSB1","model":"mt5700m-cn","instance":"boot-1","transport":"dedicated","channelReady":true,"identityVerified":true}),
        ),
        calls: Mutex::new(vec![]),
    });
    let c = ModemControls::new(
        d.clone(),
        Arc::new(Mutex::new(Security::default())),
        Instant::now(),
    );
    (d, c)
}
#[test]
fn standalone_radio_support_is_not_gated_by_shared() {
    let (d, _) = setup();
    assert!(RadioControls::supported(&d.row.lock().unwrap()));
    d.row.lock().unwrap()["identityVerified"] = json!(false);
    assert!(!RadioControls::supported(&d.row.lock().unwrap()));
}
#[test]
fn a_read_only_provider_cannot_be_enabled_for_writes_by_the_console_switch() {
    let (d, c) = setup();
    d.row.lock().unwrap()["controlReadOnly"] = json!(true);
    c.unsafe_at.store(true, Ordering::Release);
    assert!(!RadioControls::supported(&d.row.lock().unwrap()));
    assert!(c.prepare("m1", "AT+CFUN=1", "owner").is_err());
    assert!(c.prepare("m1", "AT+CSQ", "owner").is_ok());
    assert!(d.calls.lock().unwrap().is_empty());
}
#[test]
fn console_read_confirmation_binds_instance_and_is_one_use() {
    let (d, c) = setup();
    let p = c.prepare("m1", "AT+CSQ", "owner").unwrap();
    assert_eq!(p["readOnly"], true);
    d.row.lock().unwrap()["instance"] = json!("boot-2");
    assert!(
        c.execute("m1", "AT+CSQ", p["confirmation"].as_str().unwrap(), "owner")
            .is_err()
    );
    assert!(d.calls.lock().unwrap().is_empty());
    let p = c.prepare("m1", "AT+CSQ", "owner").unwrap();
    c.execute("m1", "AT+CSQ", p["confirmation"].as_str().unwrap(), "owner")
        .unwrap();
    assert!(
        c.execute("m1", "AT+CSQ", p["confirmation"].as_str().unwrap(), "owner")
            .is_err()
    );
    assert_eq!(d.calls.lock().unwrap().len(), 1);
}
#[test]
fn unsafe_disabled_by_default_and_rechecked_at_execution() {
    let (d, c) = setup();
    assert!(c.prepare("m1", "AT+CFUN=1", "owner").is_err());
    c.unsafe_at.store(true, Ordering::Release);
    let p = c.prepare("m1", "AT+CFUN=1", "owner").unwrap();
    c.unsafe_at.store(false, Ordering::Release);
    assert!(
        c.execute(
            "m1",
            "AT+CFUN=1",
            p["confirmation"].as_str().unwrap(),
            "owner"
        )
        .is_err()
    );
    assert!(d.calls.lock().unwrap().is_empty());
}
