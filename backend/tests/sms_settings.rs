#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::Instant,
};
use wlmb_server::{
    PanelError, Result, control_runtime::ModemControls, sampler::Source, security::Security,
};
struct Device {
    row: Mutex<Value>,
    state: Mutex<[u8; 3]>,
    calls: Mutex<Vec<String>>,
    fail: bool,
}
impl Source for Device {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![self.row.lock().unwrap().clone()])
    }
    fn query(&self, _: &str, cmd: &str, _: &str, _: &str) -> Result<String> {
        self.calls.lock().unwrap().push(cmd.into());
        if cmd == "AT^IMSSWITCH?" {
            let s = *self.state.lock().unwrap();
            return Ok(format!("^IMSSWITCH: {},{},{}\nOK\n", s[0], s[1], s[2]));
        }
        if cmd == "AT^IMSSWITCH=1,1,0" {
            if self.fail {
                return Ok("ERROR\n".into());
            }
            *self.state.lock().unwrap() = [1, 1, 0];
            return Ok("OK\n".into());
        }
        Err(PanelError::new("UNEXPECTED_TEST_COMMAND", 422))
    }
}
fn setup(fail: bool) -> (Arc<Device>, Arc<ModemControls>) {
    let d = Arc::new(Device {
        row: Mutex::new(
            json!({"id":"m","model":"mt5700m-cn","port":"/dev/ttyUSB1","instance":"one","shared":true}),
        ),
        state: Mutex::new([0, 1, 0]),
        calls: Mutex::new(vec![]),
        fail,
    });
    let c = ModemControls::new(
        d.clone(),
        Arc::new(Mutex::new(Security::default())),
        Instant::now(),
    );
    (d, c)
}
#[test]
fn ims_switch_requires_opt_in_and_preserves_other_flags_without_reset() {
    let (d, c) = setup(false);
    assert!(c.sms_prepare("m", true, "owner").is_err());
    assert!(d.calls.lock().unwrap().is_empty());
    c.unsafe_at.store(true, Ordering::Release);
    let p = c.sms_prepare("m", true, "owner").unwrap();
    assert_eq!(p["command"], "AT^IMSSWITCH=1,1,0");
    let nonce = p["confirmation"].as_str().unwrap();
    let result = c.sms_execute("m", true, nonce, "owner").unwrap();
    assert_eq!(result["settings"]["enabled"], true);
    assert_eq!(result["deliveryVerified"], false);
    assert!(c.sms_execute("m", true, nonce, "owner").is_err());
    let calls = d.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|s| s.starts_with("AT^IMSSWITCH="))
            .count(),
        1
    );
    assert!(
        calls
            .iter()
            .all(|s| s == "AT^IMSSWITCH?" || s == "AT^IMSSWITCH=1,1,0")
    );
}
#[test]
fn changed_ims_state_instance_owner_or_permission_blocks_write() {
    for change in 0..4 {
        let (d, c) = setup(false);
        c.unsafe_at.store(true, Ordering::Release);
        let p = c.sms_prepare("m", true, "owner").unwrap();
        match change {
            0 => *d.state.lock().unwrap() = [0, 0, 0],
            1 => d.row.lock().unwrap()["instance"] = json!("two"),
            2 => {}
            _ => c.unsafe_at.store(false, Ordering::Release),
        }
        assert!(
            c.sms_execute(
                "m",
                true,
                p["confirmation"].as_str().unwrap(),
                if change == 2 { "other" } else { "owner" }
            )
            .is_err()
        );
        assert!(
            !d.calls
                .lock()
                .unwrap()
                .iter()
                .any(|s| s.starts_with("AT^IMSSWITCH="))
        );
    }
}
#[test]
fn unsupported_model_is_not_queried_and_failed_write_is_not_replayed() {
    let (d, c) = setup(false);
    c.unsafe_at.store(true, Ordering::Release);
    d.row.lock().unwrap()["model"] = json!("rm520n-gl");
    assert_eq!(
        c.sms_prepare("m", true, "owner").unwrap_err().code,
        "SMS_SETTINGS_UNSUPPORTED"
    );
    assert!(d.calls.lock().unwrap().is_empty());
    let (d, c) = setup(true);
    c.unsafe_at.store(true, Ordering::Release);
    let p = c.sms_prepare("m", true, "owner").unwrap();
    let nonce = p["confirmation"].as_str().unwrap();
    assert_eq!(
        c.sms_execute("m", true, nonce, "owner").unwrap_err().code,
        "SMS_SETTINGS_UNCONFIRMED"
    );
    assert!(c.sms_execute("m", true, nonce, "owner").is_err());
    assert_eq!(
        d.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.starts_with("AT^IMSSWITCH="))
            .count(),
        1
    );
}
