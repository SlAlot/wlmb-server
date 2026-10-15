#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::json;
use std::sync::{Arc, Mutex, atomic::AtomicBool};
use std::time::Instant;
use wlmb_server::{Result, radio_controls::RadioControls, sampler::Source, security::Security};
use wlmb_server::{controls, lock_protocol as codec, quectel_lock as q};

#[test]
fn platform_grammars_never_inherit_by_brand_or_port_name() {
    let nr = json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":0,"scsType":1}]});
    let lte = json!({"rat":"LTE","mode":"cell","entries":[{"band":3,"arfcn":1300,"pci":12}]});
    for (vendor, models) in wlmb_server::catalog::MODELS {
        for model in models.split_whitespace() {
            if let Some(platform) = q::platform(model) {
                assert_eq!(*vendor, "quectel");
                assert!(codec::cell_supported(model));
                assert_eq!(codec::max_cells(model), 1);
                assert!(codec::band_supported(model));
                assert_eq!(
                    codec::command(model, &lte).unwrap(),
                    if platform == "lte" {
                        "AT+QNWLOCK=\"common/lte\",2,1300,12"
                    } else {
                        "AT+QNWLOCK=\"common/4g\",1,1300,12"
                    }
                );
                if platform == "lte" {
                    assert!(codec::command(model, &nr).is_err());
                } else {
                    assert_eq!(
                        codec::command(model, &nr).unwrap(),
                        "AT+QNWLOCK=\"common/5g\",0,633984,30,78"
                    );
                }
            }
        }
    }
    for model in [
        "unknown",
        "fm160-cn",
        "rg801",
        "rm500u-cn",
        "RM520N-unlisted",
    ] {
        assert!(q::platform(model).is_none());
    }
    controls::validate_preset("rm520n-gl", &nr).unwrap();
}
#[test]
fn nr_pci_zero_is_a_real_lock_not_unlocked_and_scs_is_compared() {
    let state = codec::parse(
        "rm520n-gl",
        "+QNWLOCK: \"common/5g\",0,633984,30,78\nOK\n",
        "NR",
    )
    .unwrap();
    let mut op = json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":0,"scsType":1}]});
    assert_eq!(state["mode"], 2);
    assert!(codec::matched("rm520n-gl", &state, &op));
    op["entries"][0]["scsType"] = json!(2);
    assert!(!codec::matched("rm520n-gl", &state, &op));
    let unlocked = codec::parse("rm520n-gl", "+QNWLOCK: \"common/5g\",0\nOK", "NR").unwrap();
    assert!(codec::matched(
        "rm520n-gl",
        &unlocked,
        &json!({"rat":"NR","mode":"auto","entries":[]})
    ));
}
#[test]
fn legacy_frequency_lock_is_not_mistaken_for_pci_zero_cell_lock() {
    let frequency = codec::parse("ec25", "+QNWLOCK: \"common/lte\",1,1300,0\nOK", "LTE").unwrap();
    assert_eq!(frequency["mode"], 1);
    assert!(frequency["entries"][0]["pci"].is_null());
    assert!(!codec::matched(
        "ec25",
        &frequency,
        &json!({"rat":"LTE","mode":"cell","entries":[{"arfcn":1300,"pci":0}]})
    ));
    let cell = codec::parse("ec25", "+QNWLOCK: \"common/lte\",2,1300,0\nOK", "LTE").unwrap();
    assert_eq!(cell["mode"], 2);
}
#[test]
fn different_firmware_variant_and_malformed_reply_are_never_guessed() {
    assert_eq!(
        codec::parse(
            "rm520n-gl",
            "+QNWLOCK: \"common/5g\",1,505,633984,30,78\nOK",
            "NR"
        )
        .unwrap_err()
        .code,
        "LOCK_VARIANT_UNSUPPORTED"
    );
    for raw in [
        "OK",
        "+QNWLOCK: \"common/5g\",505,633984,31,78\nOK",
        "+QNWLOCK: \"common/5g\",505,633984,30,28\nOK",
        "+QNWLOCK: \"common/5g\",1008,633984,30,78\nOK",
        "+QNWLOCK: \"common/5g\",505,633984,30,78\nOK\nERROR",
    ] {
        assert!(codec::parse("rm520n-gl", raw, "NR").is_err());
    }
    let op = json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":"633984;AT","pci":505,"scsType":1}]});
    assert!(codec::command("rm520n-gl", &op).is_err());
    assert!(codec::command("rm520n-gl", &json!({"rat":"NR","mode":"cell","entries":[]})).is_err());
}
struct Device {
    model: &'static str,
    shared: bool,
    locked: Mutex<bool>,
    calls: Mutex<Vec<String>>,
}
impl Source for Device {
    fn modems(&self) -> Result<Vec<serde_json::Value>> {
        Ok(vec![
            json!({"id":"m1","port":"/dev/ttyUSB1","model":self.model,"instance":"instance1","shared":self.shared,"transport":if self.shared{"shared"}else{"dedicated"},"channelReady":true,"identityVerified":true}),
        ])
    }
    fn query(&self, _: &str, cmd: &str, _: &str, _: &str) -> Result<String> {
        self.calls.lock().unwrap().push(cmd.into());
        if cmd.starts_with("AT+QNWPREFCFG=") || cmd == "AT+QCFG=\"band\"" {
            return Err(wlmb_server::PanelError::new("AT_UNSUPPORTED", 422));
        }
        let rat = if self.model == "em12" { "LTE" } else { "NR" };
        if cmd == codec::query(self.model, rat) {
            let suffix = if rat == "NR" {
                "\"common/5g\""
            } else {
                "\"common/lte\""
            };
            return Ok(format!(
                "+QNWLOCK: {suffix},{}\nOK",
                if *self.locked.lock().unwrap() {
                    if rat == "NR" {
                        "505,633984,30,78"
                    } else {
                        "2,1300,12"
                    }
                } else {
                    "0"
                }
            ));
        }
        if self.model != "em12" && cmd == codec::query(self.model, "LTE") {
            return Ok("+QNWLOCK: \"common/4g\",0\nOK".into());
        }
        let expected = if rat == "NR" {
            "AT+QNWLOCK=\"common/5g\",505,633984,30,78"
        } else {
            "AT+QNWLOCK=\"common/lte\",2,1300,12"
        };
        assert_eq!(cmd, expected);
        *self.locked.lock().unwrap() = true;
        Ok("OK\n".into())
    }
}
#[test]
fn actual_controller_has_the_same_typed_readback_and_one_use_write_in_both_modes() {
    for model in ["rm520n-gl", "em12"] {
        for shared in [false, true] {
            let source = Arc::new(Device {
                model,
                shared,
                locked: Mutex::new(false),
                calls: Mutex::new(vec![]),
            });
            let control = RadioControls::new(
                source.clone(),
                Arc::new(Mutex::new(Security::default())),
                Arc::new(AtomicBool::new(true)),
                Instant::now(),
            );
            let op = if model == "em12" {
                json!({"rat":"LTE","mode":"cell","entries":[{"band":3,"arfcn":1300,"pci":12}]})
            } else {
                json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":505,"scsType":1}]})
            };
            let prepared = control.prepare("m1", &op, "owner").unwrap();
            let result = control
                .execute(
                    "m1",
                    &op,
                    prepared["revision"].as_str().unwrap(),
                    prepared["confirmation"].as_str().unwrap(),
                    "owner",
                )
                .unwrap();
            assert_eq!(result["savedVerified"], true);
            assert_eq!(result["activation"], "unverified");
            assert!(
                control
                    .execute(
                        "m1",
                        &op,
                        prepared["revision"].as_str().unwrap(),
                        prepared["confirmation"].as_str().unwrap(),
                        "owner"
                    )
                    .is_err()
            );
            let calls = source.calls.lock().unwrap();
            assert_eq!(
                calls
                    .iter()
                    .filter(|cmd| cmd.ends_with("505,633984,30,78") || cmd.ends_with("2,1300,12"))
                    .count(),
                1
            );
            if model == "em12" {
                assert!(!calls.iter().any(|cmd| cmd.contains("common/5g")));
            }
        }
    }
}
