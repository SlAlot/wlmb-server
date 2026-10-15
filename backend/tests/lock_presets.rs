#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::json;
use wlmb_server::controls::{lock_command, presets, validate_preset};
#[test]
fn exact_model_presets_do_not_offer_the_entire_standard_or_other_skus() {
    let p = presets("MT5700M_CN");
    assert_eq!(p["NR"], json!([1, 3, 5, 8, 28, 41, 78, 79]));
    assert_eq!(p["LTE"], json!([1, 3, 5, 8, 34, 38, 39, 40, 41]));
    assert!(presets("unknown")["NR"].as_array().unwrap().is_empty());
    assert_eq!(presets("MT5710-CN")["lockProtocolSupported"], false);
    assert_eq!(p["NRSA"], json!([1, 28, 41, 78, 79]));
    assert_eq!(p["NRNSA"], json!([41, 78, 79]));
    assert_eq!(p["source"], "reviewed-model-supplement");
    assert_eq!(p["supplement"]["planeSupportVerified"], false);
    for rat in ["NR", "LTE"] {
        for band in p[rat].as_array().unwrap() {
            let operation = json!({"rat":rat,"mode":"band","entries":[{"band":band}]});
            validate_preset("mt5700m-cn", &operation).unwrap();
            lock_command(&operation).unwrap();
        }
    }
}
#[test]
fn mt5700_supplement_is_scoped_and_does_not_rewrite_upstream_facts() {
    let facts: serde_json::Value =
        serde_json::from_str(include_str!("../../docs/model-bands.json")).unwrap();
    let rows = facts["profiles"].as_array().unwrap();
    for model in ["mt5700m-cn", "MT5700M_CN"] {
        for band in [3, 5, 8] {
            let op = json!({"rat":"NR","mode":"band","entries":[{"band":band}]});
            validate_preset(model, &op).unwrap();
            assert_eq!(
                lock_command(&op).unwrap(),
                format!("AT^NRFREQLOCK=3,1,1,\"{band}\"")
            );
        }
    }
    for row in rows {
        let model = row["model"].as_str().unwrap();
        let p = presets(model);
        if model == "mt5700m-cn" {
            assert_eq!(row["NRSA"], json!([1, 28, 41, 78, 79]));
            continue;
        }
        assert!(p.get("supplement").is_none(), "{model}");
        let expected: std::collections::BTreeSet<_> = rows
            .iter()
            .filter(|r| r["model"] == model)
            .flat_map(|r| {
                r["NRSA"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .chain(r["NRNSA"].as_array().unwrap())
            })
            .map(|n| n.as_u64().unwrap())
            .collect();
        assert!(
            p["NR"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| expected.contains(&n.as_u64().unwrap())),
            "{model}"
        );
    }
}
#[test]
fn unsupported_band_and_mismatched_cell_frequency_cannot_reach_transport() {
    let unsupported = json!({"rat":"NR","mode":"band","entries":[{"band":77}]});
    assert_eq!(
        validate_preset("mt5700m-cn", &unsupported)
            .unwrap_err()
            .code,
        "LOCK_BAND_UNSUPPORTED"
    );
    let mismatch = json!({"rat":"NR","mode":"cell","entries":[{"band":28,"arfcn":633984,"pci":505,"scsType":1}]});
    assert!(lock_command(&mismatch).is_err());
    let auto = json!({"rat":"LTE","mode":"auto","entries":[]});
    validate_preset("mt5700m-cn", &auto).unwrap();
    assert!(validate_preset("unverified", &auto).is_err());
}
#[test]
fn supplemented_bands_pass_real_confirmation_and_readback_in_both_channel_modes() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use wlmb_server::{Result, radio_controls::RadioControls, sampler::Source, security::Security};
    struct Fixture {
        shared: bool,
        band: Mutex<Option<u32>>,
        writes: AtomicUsize,
    }
    impl Source for Fixture {
        fn modems(&self) -> Result<Vec<serde_json::Value>> {
            Ok(vec![
                json!({"id":"m1","port":"/dev/ttyUSB1","model":"mt5700m-cn","shared":self.shared,"instance":1,"transport":"dedicated","channelReady":true,"identityVerified":true}),
            ])
        }
        fn query(&self, _: &str, cmd: &str, _: &str, _: &str) -> Result<String> {
            if cmd == "AT^NRFREQLOCK?" {
                return Ok(match *self.band.lock().unwrap() {
                    Some(band) => format!("^NRFREQLOCK: 3\n1,1\n{band}\nOK\n"),
                    None => "^NRFREQLOCK: 0\nOK\n".into(),
                });
            }
            if cmd == "AT^LTEFREQLOCK?" {
                return Ok("^LTEFREQLOCK: 0\nOK\n".into());
            }
            let band: u32 = cmd
                .strip_prefix("AT^NRFREQLOCK=3,1,1,\"")
                .and_then(|v| v.strip_suffix('"'))
                .unwrap()
                .parse()
                .unwrap();
            assert!([3, 5, 8].contains(&band));
            *self.band.lock().unwrap() = Some(band);
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok("OK\n".into())
        }
    }
    for shared in [false, true] {
        for band in [3, 5, 8] {
            let source = Arc::new(Fixture {
                shared,
                band: Mutex::new(None),
                writes: AtomicUsize::new(0),
            });
            let control = RadioControls::new(
                source.clone(),
                Arc::new(Mutex::new(Security::default())),
                Arc::new(AtomicBool::new(true)),
                std::time::Instant::now(),
            );
            let op = json!({"rat":"NR","mode":"band","entries":[{"band":band}]});
            let p = control.prepare("m1", &op, "owner").unwrap();
            assert_eq!(source.writes.load(Ordering::SeqCst), 0);
            let result = control
                .execute(
                    "m1",
                    &op,
                    p["revision"].as_str().unwrap(),
                    p["confirmation"].as_str().unwrap(),
                    "owner",
                )
                .unwrap();
            assert_eq!(result["savedVerified"], true);
            assert_eq!(source.writes.load(Ordering::SeqCst), 1);
            assert!(
                control
                    .execute(
                        "m1",
                        &op,
                        p["revision"].as_str().unwrap(),
                        p["confirmation"].as_str().unwrap(),
                        "owner"
                    )
                    .is_err()
            );
            assert_eq!(source.writes.load(Ordering::SeqCst), 1);
        }
    }
}
