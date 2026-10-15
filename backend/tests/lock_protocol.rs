#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};
use wlmb_server::{
    PanelError, Result, lock_protocol as codec, radio_controls::RadioControls, sampler::Source,
    security::Security,
};
#[test]
fn unisoc_is_not_qualcomm_or_a_brand_fallback() {
    for model in ["RG200U-CN", "RM500U-CN", "RM500U-CNV", "RM500U-EA"] {
        assert!(codec::cell_supported(model));
        assert!(codec::band_supported(model));
        let operation =
            json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":505}]});
        wlmb_server::controls::validate_preset(model, &operation).unwrap();
        assert_eq!(
            codec::command(model, &operation).unwrap(),
            "AT+QNWLOCK=\"common/5g\",1,633984,505"
        );
        assert_eq!(
            codec::command(model, &json!({"rat":"LTE","mode":"auto","entries":[]})).unwrap(),
            "AT+QNWLOCK=\"common/lte\",0"
        );
    }
    assert!(codec::cell_supported("RM520N-GL"));
    assert!(codec::cell_supported("FM650-CN"));
    assert!(codec::command("unknown", &json!({"rat":"NR","mode":"auto","entries":[]})).is_err());
}
#[test]
fn readback_reports_actual_fields_without_guessing_band_or_scs() {
    let state = codec::parse(
        "rm500u-cn",
        "+QNWLOCK: \"common/5g\",633984,505\nOK\n",
        "NR",
    )
    .unwrap();
    assert_eq!(state["mode"], 2);
    assert!(state["entries"][0]["band"].is_null());
    assert!(state["entries"][0]["scsType"].is_null());
    assert_eq!(state["entries"][0]["bandCandidates"], json!(["n77", "n78"]));
    assert_eq!(state["persistent"], false);
    assert!(codec::matched(
        "rm500u-cn",
        &state,
        &json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":505}]})
    ));
    assert!(!codec::matched(
        "rm500u-cn",
        &state,
        &json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":506}]})
    ));
    for raw in [
        "+QNWLOCK: \"common/5g\",633984,505",
        "+QNWLOCK: \"common/lte\",633984,505\nOK",
        "+QNWLOCK: \"common/5g\",633984,1008\nOK",
        "OK\nERROR",
        "+QNWLOCK: \"common/5g\",633984,505\n+QNWLOCK: \"common/5g\",633984,505\nOK",
    ] {
        assert!(codec::parse("rm500u-cn", raw, "NR").is_err());
    }
}
#[test]
fn malformed_or_cross_band_operations_fail_before_a_write() {
    for operation in [
        json!({"rat":"NR","mode":"cell","entries":[{"arfcn":"633984;AT","pci":505}]}),
        json!({"rat":"NR","mode":"cell","entries":[{"arfcn":633984,"pci":1008}]}),
        json!({"rat":"NR","mode":"band","entries":[{"band":78}]}),
        json!({"rat":"NR","mode":"auto","entries":[{"arfcn":633984,"pci":505}]}),
    ] {
        assert!(codec::command("rm500u-cn", &operation).is_err());
    }
    assert!(
        wlmb_server::controls::validate_preset(
            "rm500u-cn",
            &json!({"rat":"NR","mode":"cell","entries":[{"band":28,"arfcn":633984,"pci":505}]})
        )
        .is_err()
    );
}
struct Modem {
    calls: Mutex<Vec<String>>,
    locked: Mutex<bool>,
    reject: bool,
}
impl Source for Modem {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![
            json!({"id":"m1","model":"rm500u-cn","port":"/dev/ttyUSB1","shared":true,"instance":1}),
        ])
    }
    fn query(&self, _: &str, cmd: &str, _: &str, _: &str) -> Result<String> {
        self.calls.lock().unwrap().push(cmd.into());
        match cmd {
            "AT+QNWLOCK=\"common/5g\"" => Ok(if *self.locked.lock().unwrap() {
                "+QNWLOCK: \"common/5g\",633984,505\nOK\n"
            } else {
                "OK\n"
            }
            .into()),
            "AT+QNWLOCK=\"common/lte\"" => Ok("OK\n".into()),
            "AT+QNWLOCK=\"common/5g\",1,633984,505" => {
                if self.reject {
                    Err(PanelError::new("AT_UNSUPPORTED", 422))
                } else {
                    *self.locked.lock().unwrap() = true;
                    Ok("OK\n".into())
                }
            }
            _ if cmd.starts_with("AT+QNWPREFCFG=") => Err(PanelError::new("AT_UNSUPPORTED", 422)),
            _ => panic!("Unexpected {cmd}"),
        }
    }
}
#[test]
fn typed_confirmation_and_readback_work_in_the_actual_controller() {
    for reject in [false, true] {
        let source = Arc::new(Modem {
            calls: Mutex::new(vec![]),
            locked: Mutex::new(false),
            reject,
        });
        let control = RadioControls::new(
            source.clone(),
            Arc::new(Mutex::new(Security::default())),
            Arc::new(AtomicBool::new(true)),
            Instant::now(),
        );
        let op = json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":505}]});
        let prepared = control.prepare("m1", &op, "owner").unwrap();
        assert!(prepared["warning"].as_str().unwrap().contains("不持久"));
        let result = control.execute(
            "m1",
            &op,
            prepared["revision"].as_str().unwrap(),
            prepared["confirmation"].as_str().unwrap(),
            "owner",
        );
        if reject {
            assert_eq!(result.unwrap_err().code, "LOCK_REJECTED");
        } else {
            let result = result.unwrap();
            assert_eq!(result["savedVerified"], true);
            assert_eq!(result["persistent"], false);
        }
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
        assert_eq!(
            source
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|cmd| cmd.ends_with(",1,633984,505"))
                .count(),
            1
        );
    }
}
