#![allow(clippy::unwrap_used)]
// 模板协议与控制器模拟回归，不操作设备、不执行真实锁定。
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};
use wlmb_server::{
    PanelError, Result, band_locks as bands, catalog, radio_controls::RadioControls,
    sampler::Source, security::Security,
};
#[path = "support/band_templates.rs"]
mod cases;
const MODELS: [&str; 4] = ["em9190", "em9291", "mc7354", "mc7355"];
fn replies(model: &str, locked: bool) -> BTreeMap<String, String> {
    let lte = catalog::platform(model) == "lte";
    BTreeMap::from([
        ("AT!BAND?".into(), cases::current(lte, locked)),
        ("AT!BAND=?".into(), cases::available(lte)),
    ])
}
fn operation() -> Value {
    json!({"rat":"NR","mode":"band","target":"band","entries":[{"band":78}]})
}
#[test]
fn templates_keep_sa_nsa_gw_and_firmware_inventory_separate() {
    for model in MODELS {
        assert!(bands::supported(model));
        let state = bands::parse(model, &replies(model, false)).unwrap();
        assert_eq!(state["groups"]["LTE"]["availableBands"], json!([1, 3, 5]));
        assert_eq!(state["wire"]["templateMasks"]["GW"][0], "0000000000000001");
        if catalog::platform(model) == "lte" {
            assert!(state["groups"]["NR"].is_null());
            continue;
        }
        assert_eq!(state["groups"]["NR"]["availableBands"], json!([41, 78, 79]));
        let op = operation();
        let commands = bands::commands(model, &op, &state).unwrap();
        assert_eq!(commands.len(), 5);
        assert_eq!(commands[0], "AT!BAND=0F,1,\"Custom\",0,0000000000000001");
        assert_eq!(
            commands[1],
            "AT!BAND=0F,1,\"Custom\",1,0000000000000005,0000000000000000"
        );
        assert_eq!(
            commands[2],
            "AT!BAND=0F,1,\"Custom\",3,0000000000000000,0000000000002000"
        );
        assert_eq!(
            commands[3],
            "AT!BAND=0F,1,\"Custom\",4,0000000000000000,0000000000002000"
        );
        assert_eq!(commands[4], "AT!BAND=0F");
        let after = bands::parse(model, &replies(model, true)).unwrap();
        assert!(bands::matched_transition(model, &state, &after, &op));
        let mut changed = after.clone();
        changed["wire"]["templateMasks"]["GW"][0] = json!("0000000000000000");
        assert!(!bands::matched_transition(model, &state, &changed, &op));
        let mut changed = after.clone();
        changed["groups"]["NR_NSA"]["selectedBands"] = json!([41]);
        assert!(!bands::matched_transition(model, &state, &changed, &op));
    }
}
#[test]
fn incomplete_ambiguous_or_unsupported_templates_never_enable_a_write() {
    for (command, old, new) in [
        ("AT!BAND?", "0 - GW: 0000000000000001\n", ""),
        (
            "AT!BAND?",
            "1 - LTE: 0000000000000005 0000000000000000",
            "1 - LTE: ZZZ 0",
        ),
        ("AT!BAND?", "OK", "ERROR"),
        (
            "AT!BAND=?",
            "Available:",
            "Available:".trim_end_matches(':'),
        ),
        (
            "AT!BAND=?",
            "0000000000002000 - n78",
            "0000000000002000 - n129",
        ),
        (
            "AT!BAND=?",
            "0000000000002000 - n78",
            "0000000000002000 - n41",
        ),
    ] {
        let mut r = replies("em9190", false);
        r.insert(command.into(), r[command].replace(old, new));
        assert!(bands::parse("em9190", &r).is_err(), "{command} {new}");
    }
    let mut r = replies("em9190", false);
    r.insert(
        "AT!BAND?".into(),
        r["AT!BAND?"].replace("OK", "1 - LTE: 1 0\nOK"),
    );
    assert!(bands::parse("em9190", &r).is_err());
    let state = bands::parse("em9190", &replies("em9190", false)).unwrap();
    let invalid = json!({"rat":"NR","mode":"band","entries":[{"band":257}]});
    assert!(bands::commands("em9190", &invalid, &state).is_err());
    for cmd in ["AT!BAND?", "AT!BAND=?"] {
        assert!(wlmb_server::security::read_only(cmd));
    }
    assert!(!wlmb_server::security::read_only("AT!BAND=0F"));
}
struct Device {
    model: &'static str,
    shared: bool,
    locked: Mutex<bool>,
    writes: Mutex<Vec<String>>,
    fail_at: usize,
}
impl Source for Device {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![
            json!({"id":"m1","model":self.model,"instance":1,"port":"/dev/ttyUSB1","shared":self.shared,"transport":"dedicated","channelReady":true,"identityVerified":true}),
        ])
    }
    fn query(&self, _: &str, command: &str, _: &str, _: &str) -> Result<String> {
        if let Some(reply) = replies(self.model, *self.locked.lock().unwrap()).get(command) {
            return Ok(reply.clone());
        }
        if command.starts_with("AT!BAND=") {
            let count = {
                let mut writes = self.writes.lock().unwrap();
                writes.push(command.into());
                writes.len()
            };
            if count == self.fail_at {
                return Err(PanelError::new("AT_FAILED", 503));
            }
            if command == "AT!BAND=0F" {
                *self.locked.lock().unwrap() = true;
            }
            return Ok("OK\n".into());
        }
        panic!("意外的模组指令：{command}")
    }
}
#[test]
fn template_controller_has_the_same_confirmation_and_readback_in_both_modes() {
    for model in &MODELS[..2] {
        for shared in [true, false] {
            for fail_at in [0, 1, 3, 5] {
                let device = Arc::new(Device {
                    model,
                    shared,
                    locked: Mutex::new(false),
                    writes: Mutex::new(vec![]),
                    fail_at,
                });
                let control = RadioControls::new(
                    device.clone(),
                    Arc::new(Mutex::new(Security::default())),
                    Arc::new(AtomicBool::new(true)),
                    Instant::now(),
                );
                let op = operation();
                let prepared = control.prepare("m1", &op, "owner").unwrap();
                assert!(prepared["warning"].as_str().unwrap().contains("其他制式"));
                let revision = prepared["revision"].as_str().unwrap();
                let token = prepared["confirmation"].as_str().unwrap();
                let result = control.execute("m1", &op, revision, token, "owner");
                if fail_at != 0 {
                    let code = if fail_at == 1 {
                        "LOCK_RESULT_UNKNOWN"
                    } else {
                        "LOCK_PARTIAL"
                    };
                    assert_eq!(result.unwrap_err().code, code);
                    assert_eq!(device.writes.lock().unwrap().len(), fail_at);
                } else {
                    assert_eq!(result.unwrap()["savedVerified"], true);
                    assert_eq!(device.writes.lock().unwrap().len(), 5);
                }
                let count = device.writes.lock().unwrap().len();
                assert!(
                    control
                        .execute("m1", &op, revision, token, "owner")
                        .is_err()
                );
                assert_eq!(device.writes.lock().unwrap().len(), count);
            }
        }
    }
}
