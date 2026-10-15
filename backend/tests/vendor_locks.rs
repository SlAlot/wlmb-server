#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};
use wlmb_server::{
    PanelError, Result, band_locks as bands, lock_protocol as cells, radio_controls::RadioControls,
    sampler::Source, security::Security,
};
fn op(rat: &str, band: u64) -> Value {
    json!({"rat":rat,"mode":"band","entries":[{"band":band}],"target":"band"})
}
fn replies(model: &str) -> BTreeMap<String, String> {
    bands::queries(model).into_iter().map(|cmd| {
        let response=if cmd.starts_with("AT+QNWPREFCFG=")||cmd.starts_with("AT+CSYSSEL=") {
            let selector=cmd.split_once('=').unwrap().1;
            format!("+{}: {selector},{}\nOK\n",if cmd.starts_with("AT+Q"){"QNWPREFCFG"}else{"CSYSSEL"},if selector=="\"lte_band\""{"1:3"}else{"41:78"})
        }else{match cmd.as_str(){
            "AT+QCFG=\"band\""=>"+QCFG: \"band\",0x15,0x5,0x8\nOK\n".into(),
            "AT^BAND_PREF?"|"AT+BAND_PREF?"=>"LTE,Enable Bands:1,3\nLTE,Disable Bands:5\nNR5G_SA,Enable Bands:78\nNR5G_SA,Disable Bands:41\nNR5G_NSA,Enable Bands:78\nNR5G_NSA,Disable Bands:41\nOK\n".into(),
            "AT+ZBAND?"=>if model=="me3630-w" {"LTE:1,3\nOK\n"}else{"LTE:1,3\nNR5G:41,78\nOK\n"}.into(),
            "AT+ZBAND=?"=>if model=="me3630-w" {"LTE:(1,3,5)\nOK\n"}else{"LTE:(1,3,5)\nNR5G:(41,78,79)\nOK\n"}.into(),
            "AT+GTACT?"=>"+GTACT:17,6,6,103,5041,5078\nOK\n".into(),
            "AT+GTACT=?"=>"+GTACT:(1,2,4,17),(6),(6),(0),(1,2),(101,103,108),(0),(0),(5041,5078,5079)\nOK\n".into(),
            "AT#BND?"=>"#BND:0,22,5,0,0,2000,0,2000\nOK\n".into(),
            "AT#BND=?"=>"#BND:(0),(22),(5),(0),(10000000000),(2000),(10000000000),(2000)\nOK\n".into(),
            "AT*BAND?"=>"*BAND:5,7,8,2,5,0,2,1\nOK\n".into(),
            "AT+CNBP?"=>"+CNBP:0x15,0x5,0x2,0\nOK\n".into(),
            "AT+NWSETBAND?"=>"+NWSETBAND:2\n3,2,B1,B3\n6,2,N41,N78\nOK\n".into(),
            "AT+NWSETBAND=?"=>"+NWSETBAND:0,B1,B3,N41,N78\nOK\n".into(),
            _=>panic!("unexpected {cmd}"),
        }};
        (cmd,response)
    }).collect()
}
#[test]
fn band_families_use_typed_commands_and_keep_sa_nsa_separate() {
    for (model, rat, band, expected) in [
        ("rm520n-gl", "NR", 78, "AT+QNWPREFCFG=\"nr5g_band\",78"),
        (
            "rm500u-cn",
            "NR_NSA",
            78,
            "AT+QNWPREFCFG=\"nsa_nr5g_band\",78",
        ),
        ("em160r-gl", "LTE", 3, "AT+QNWPREFCFG=\"lte_band\",3"),
        ("em05", "LTE", 3, "AT+QCFG=\"band\",0x15,0x4,0x8"),
        (
            "simcom-sim8200ea-m2",
            "NR",
            78,
            "AT+CSYSSEL=\"nr5g_band\",78",
        ),
        ("t99w368", "NR", 78, "AT+BAND_PREF=NR5G,2,78"),
        ("t99w373", "LTE", 3, "AT^BAND_PREF=LTE,2,3"),
        ("mv32-w-a", "NR", 78, "AT+BAND_PREF=NR5G,2,78"),
        ("gm800", "NR", 78, "AT+ZBAND=5,1,78"),
        ("me3630-w", "LTE", 3, "AT+ZBAND=all,all,all,4"),
        ("fm350-gl", "NR", 78, "AT+GTACT=17,6,6,103,5078"),
        ("fn990a40", "NR", 78, "AT#BND=0,22,5,0,0,2000,0,2000"),
        ("air724ug", "LTE", 3, "AT*BAND=5,7,8,0,4,0,2,1"),
        ("a7605c", "LTE", 34, "AT+CNBP=0x15,0x0000000000000000,0x2,0"),
        ("n510m", "NR", 78, "AT+NWSETBAND=6,1,78"),
    ] {
        let state = bands::parse(model, &replies(model)).unwrap();
        let operation = op(rat, band);
        assert_eq!(
            bands::commands(model, &operation, &state)
                .unwrap_or_else(|e| panic!("{model}: {e:?}; {state}")),
            vec![expected],
            "{model}"
        );
        wlmb_server::security::validate_at(expected).unwrap();
    }
}
#[test]
fn reset_is_explicit_band_target_and_does_not_generate_cell_unlock() {
    let model = "rm520n-gl";
    let reset = json!({"rat":"NR","mode":"auto","entries":[],"target":"band"});
    let commands = bands::commands(
        model,
        &reset,
        &bands::parse(model, &replies(model)).unwrap(),
    )
    .unwrap();
    assert!(commands[0].starts_with("AT+QNWPREFCFG=\"nr5g_band\","));
    assert!(!commands[0].contains("QNWLOCK"));
    assert!(!bands::operation(
        &json!({"rat":"NR","mode":"auto","entries":[]})
    ));
    for model in ["unknown", "srm825", "mt5710-cn", "rg801"] {
        assert!(!bands::supported(model));
    }
    assert!(bands::supported("em9190"));
    assert!(!cells::cell_supported("air724ug"));
}
#[test]
fn malformed_or_ambiguous_state_cannot_enable_a_write() {
    for model in [
        "rm520n-gl",
        "ec25",
        "t99w368",
        "fm350-gl",
        "fn990a40",
        "air724ug",
        "a7605c",
        "n510m",
    ] {
        let mut input = replies(model);
        for raw in input.values_mut() {
            *raw = raw.replace("OK\n", "ERROR\nOK\n");
        }
        assert!(bands::parse(model, &input).is_err(), "{model}");
        assert!(bands::commands(model, &op("LTE", 3), &json!({})).is_err());
    }
    for operation in [
        json!({"rat":"LTE","mode":"band","entries":[{"band":3},{"band":3}]}),
        json!({"rat":"LTE","mode":"band","entries":[{"band":"3;AT"}]}),
        json!({"rat":"LTE","mode":"band","entries":[],"target":"cell"}),
    ] {
        assert!(bands::validate("rm520n-gl", &operation).is_err());
    }
    let unknown = bands::parse("ec25", &replies("ec25")).unwrap();
    assert!(
        bands::commands("ec25", &op("LTE", 3), &unknown).is_err(),
        "a current mask is not proof of the full supported inventory"
    );
    let mut state = bands::parse("fm350-gl", &replies("fm350-gl")).unwrap();
    state["wire"]["availableCodes"][2] = json!([578, 5078]);
    assert!(
        bands::commands("fm350-gl", &op("NR", 78), &state).is_err(),
        "ambiguous band code must not be guessed"
    );
    let mut input = replies("air724ug");
    input.insert("AT*BAND?".into(), "*BAND:0,7,8,2,5,0,2,1\nOK\n".into());
    let state = bands::parse("air724ug", &input).unwrap();
    assert_eq!(
        bands::commands("air724ug", &op("LTE", 3), &state)
            .unwrap_err()
            .code,
        "LOCK_REQUIRES_LTE_MODE"
    );
}

#[test]
fn negotiated_full_inventory_is_auto_not_an_empty_sku_manual_selection() {
    for model in ["fm350-gl", "fn990a40", "n510m", "gm800", "me3630-w"] {
        let mut frames = replies(model);
        for raw in frames.values_mut() {
            *raw = match model {
                "fm350-gl" => {
                    raw.replace("17,6,6,103,5041,5078", "17,6,6,101,103,108,5041,5078,5079")
                }
                "fn990a40" => raw.replace(
                    "0,22,5,0,0,2000,0,2000",
                    "0,22,5,0,10000000000,2000,10000000000,2000",
                ),
                "n510m" => raw.replace("+NWSETBAND:2\n3,2,B1,B3\n6,2,N41,N78", "+NWSETBAND:0"),
                "gm800" => raw.replace("LTE:1,3\nNR5G:41,78", "LTE:1,3,5\nNR5G:41,78,79"),
                _ => raw.replace("LTE:1,3\n", "LTE:1,3,5\n"),
            };
        }
        let state = bands::parse(model, &frames).unwrap();
        assert!(
            state["groups"]
                .as_object()
                .unwrap()
                .values()
                .all(|g| g["mode"] == 0),
            "{model} full firmware range should display automatic"
        );
    }
    let mut frames = replies("gm800");
    frames.insert("AT+ZBAND?".into(), "LTE:1,3\nNR5G:41,78,99\nOK\n".into());
    assert!(
        bands::parse("gm800", &frames).is_err(),
        "a contradictory current/support template is not writable"
    );
    assert!(
        bands::parse("me3630-w", &replies("gm800")).is_err(),
        "LTE-only model cannot acquire a fake NR plane"
    );
}
#[test]
fn cell_codecs_preserve_pci_zero_and_do_not_send_network_preference() {
    let op = json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":0,"scsType":1}]});
    for (model, reply, expected) in [
        (
            "fm350-gl",
            "+GTCELLLOCK:1,1,0,633984,0,1,5078\nOK\n",
            "AT+GTCELLLOCK=1,1,0,633984,0,1,5078",
        ),
        (
            "simcom-sim8200ea-m2",
            "+C5GCELLCFG:\"pci\",0,633984,1,78\nOK\n",
            "AT+C5GCELLCFG=\"pci\",0,633984,1,78",
        ),
        (
            "gm800",
            "LTE,status:off\nNR5G,nr5g_chanel:633984,pci:0,scs:1,band:78,status:on\nOK\n",
            "AT+ZLOCKCELL=1,2,633984,0,1,78",
        ),
    ] {
        assert_eq!(cells::command(model, &op).unwrap(), expected);
        let state = cells::parse(model, reply, "NR").unwrap();
        assert!(cells::matched(model, &state, &op));
        for trailer in ["", "ERROR\n", "+CME ERROR:4\nOK\n"] {
            assert!(cells::parse(model, &reply.replace("OK\n", trailer), "NR").is_err());
        }
        assert!(!expected.contains("CNMP") && !expected.contains("CFUN"));
    }
    for model in ["fm650-cn", "fm160-cn", "nl668", "nl678"] {
        assert!(cells::cell_supported(model));
    }
    assert!(
        cells::command(
            "fm350-gl",
            &json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":1}]})
        )
        .is_err()
    );
}
#[test]
fn every_reviewed_model_band_has_a_channel_envelope_not_only_domestic_n78() {
    for model in wlmb_server::catalog::inventory()["models"]
        .as_array()
        .unwrap()
    {
        let presets = wlmb_server::band_inventory::presets(model["model"].as_str().unwrap());
        for (rat, key, table) in [
            ("NR", "NR", wlmb_server::radio::NR_RANGES),
            ("LTE", "LTE", wlmb_server::radio::LTE_RANGES),
        ] {
            for band in presets[key].as_array().unwrap() {
                assert!(
                    table
                        .iter()
                        .any(|(b, _, _)| Some(*b as u64) == band.as_u64()),
                    "{} {rat} {band}",
                    model["model"]
                );
            }
        }
    }
    let op = json!({"rat":"NR","mode":"cell","entries":[{"band":48,"arfcn":640000,"pci":0,"scsType":1}]});
    assert!(cells::command("rm520n-gl", &op).unwrap().ends_with(",48"));
    assert!(wlmb_server::radio::band_candidates("NR", 640000.).contains(&"n48".into()));
    assert!(
        wlmb_server::radio::band_candidates("NR", 2104166.)
            .iter()
            .all(|b| b != "n257")
    );
}
struct Device {
    shared: bool,
    selected: Mutex<String>,
    calls: Mutex<Vec<String>>,
    changed: Mutex<bool>,
}
impl Source for Device {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![
            json!({"id":"m1","model":"rm520n-gl","instance":1,"port":"/dev/ttyUSB1","shared":self.shared,"transport":"dedicated","channelReady":true,"identityVerified":true}),
        ])
    }
    fn query(&self, _: &str, cmd: &str, _: &str, _: &str) -> Result<String> {
        self.calls.lock().unwrap().push(cmd.into());
        if cmd.starts_with("AT+QNWLOCK=") {
            return Ok(format!(
                "+QNWLOCK:{},0\nOK\n",
                cmd.split_once('=').unwrap().1
            ));
        }
        if cmd == "AT+QNWPREFCFG=\"nr5g_band\"" {
            return Ok(format!(
                "+QNWPREFCFG: \"nr5g_band\",{}\nOK\n",
                self.selected.lock().unwrap()
            ));
        }
        if let Some(reply) = replies("rm520n-gl").get(cmd) {
            return Ok(reply.clone());
        }
        if cmd == "AT+QNWPREFCFG=\"nr5g_band\",78" {
            if *self.changed.lock().unwrap() {
                return Err(PanelError::new("SYSTEM_BUSY", 503));
            }
            *self.selected.lock().unwrap() = "78".into();
            return Ok("OK\n".into());
        }
        Err(PanelError::new("AT_UNSUPPORTED", 422))
    }
}
#[test]
fn actual_controller_uses_the_same_band_flow_in_shared_and_owned_modes() {
    for shared in [false, true] {
        let device = Arc::new(Device {
            shared,
            selected: Mutex::new("41:78".into()),
            calls: Mutex::new(vec![]),
            changed: Mutex::new(false),
        });
        let control = RadioControls::new(
            device.clone(),
            Arc::new(Mutex::new(Security::default())),
            Arc::new(AtomicBool::new(true)),
            Instant::now(),
        );
        let op = op("NR", 78);
        let p = control.prepare("m1", &op, "owner").unwrap();
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
        assert_eq!(result["activation"], "unverified");
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
        assert_eq!(
            device
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.ends_with("\",78"))
                .count(),
            1
        );
    }
}
#[test]
fn changed_readback_and_transport_failure_cannot_replay_band_writes() {
    let d = Arc::new(Device {
        shared: true,
        selected: Mutex::new("41:78".into()),
        calls: Mutex::new(vec![]),
        changed: Mutex::new(false),
    });
    let c = RadioControls::new(
        d.clone(),
        Arc::new(Mutex::new(Security::default())),
        Arc::new(AtomicBool::new(true)),
        Instant::now(),
    );
    let op = op("NR", 78);
    let p = c.prepare("m1", &op, "owner").unwrap();
    *d.selected.lock().unwrap() = "41".into();
    assert_eq!(
        c.execute(
            "m1",
            &op,
            p["revision"].as_str().unwrap(),
            p["confirmation"].as_str().unwrap(),
            "owner"
        )
        .unwrap_err()
        .code,
        "LOCK_CHANGED"
    );
    let p = c.prepare("m1", &op, "owner").unwrap();
    *d.changed.lock().unwrap() = true;
    assert_eq!(
        c.execute(
            "m1",
            &op,
            p["revision"].as_str().unwrap(),
            p["confirmation"].as_str().unwrap(),
            "owner"
        )
        .unwrap_err()
        .code,
        "LOCK_RESULT_UNKNOWN"
    );
    assert!(
        c.execute(
            "m1",
            &op,
            p["revision"].as_str().unwrap(),
            p["confirmation"].as_str().unwrap(),
            "owner"
        )
        .is_err()
    );
}

struct TypedDevice {
    model: &'static str,
    shared: bool,
    replies: Mutex<BTreeMap<String, String>>,
    writes: BTreeMap<String, (String, String)>,
    calls: Mutex<Vec<String>>,
}
impl Source for TypedDevice {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![
            json!({"id":"m1","model":self.model,"instance":1,"port":"/dev/ttyUSB1","shared":self.shared,"transport":"dedicated","channelReady":true,"identityVerified":true}),
        ])
    }
    fn query(&self, _: &str, cmd: &str, _: &str, _: &str) -> Result<String> {
        self.calls.lock().unwrap().push(cmd.into());
        let mut replies = self.replies.lock().unwrap();
        if let Some(raw) = replies.get(cmd) {
            return Ok(raw.clone());
        }
        if let Some((read_command, response)) = self.writes.get(cmd) {
            replies.insert(read_command.clone(), response.clone());
            return Ok("OK\n".into());
        }
        Err(PanelError::new("AT_UNSUPPORTED", 422))
    }
}
fn controller(device: &Arc<TypedDevice>) -> RadioControls {
    RadioControls::new(
        device.clone(),
        Arc::new(Mutex::new(Security::default())),
        Arc::new(AtomicBool::new(true)),
        Instant::now(),
    )
}
fn execute(control: &RadioControls, op: &Value, prepared: &Value) -> Result<Value> {
    control.execute(
        "m1",
        op,
        prepared["revision"].as_str().unwrap(),
        prepared["confirmation"].as_str().unwrap(),
        "owner",
    )
}
#[test]
fn dynamic_inventory_and_cell_roundtrip_are_identical_on_both_channel_modes() {
    let op = json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":0,"scsType":1}]});
    for shared in [false, true] {
        for (model, query, empty, saved) in [
            (
                "fm350-gl",
                "AT+GTCELLLOCK?",
                "+GTCELLLOCK:0\nOK\n",
                "+GTCELLLOCK:1,1,0,633984,0,1,5078\nOK\n",
            ),
            (
                "simcom-sim8200ea-m2",
                "AT+C5GCELLCFG?",
                "+C5GCELLCFG:\"unlock\"\nOK\n",
                "+C5GCELLCFG:\"pci\",0,633984,1,78\nOK\n",
            ),
            (
                "gm800",
                "AT+ZLOCKCELL?",
                "LTE,status:off\nNR5G,status:off\nOK\n",
                "LTE,status:off\nNR5G,nr5g_chanel:633984,pci:0,scs:1,band:78,status:on\nOK\n",
            ),
        ] {
            let mut frames = replies(model);
            frames.insert(query.into(), empty.into());
            if model.starts_with("simcom") {
                frames.insert("AT+CCELLCFG?".into(), "+CCELLCFG:0\nOK\n".into());
            }
            let write = cells::command(model, &op).unwrap();
            let device = Arc::new(TypedDevice {
                model,
                shared,
                replies: Mutex::new(frames),
                writes: BTreeMap::from([(write.clone(), (query.into(), saved.into()))]),
                calls: Mutex::new(vec![]),
            });
            let control = controller(&device);
            let state = control.read("m1", true).unwrap();
            assert!(
                state["presets"]["NR"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(78))
            );
            assert_eq!(
                device
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| c.as_str() == query)
                    .count(),
                1,
                "a global cell reader must not query twice"
            );
            if model == "fm350-gl" {
                assert_eq!(state["presets"]["source"], "negotiated-firmware");
            }
            let p = control.prepare("m1", &op, "owner").unwrap();
            let result = execute(&control, &op, &p).unwrap();
            assert_eq!(result["savedVerified"], true, "{model}, shared={shared}");
            assert_eq!(result["activation"], "unverified");
            assert_eq!(
                device
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| *c == &write)
                    .count(),
                1
            );
        }
    }
}
#[test]
fn global_cell_unlock_confirmation_binds_the_other_rat_and_verifies_both() {
    let query = "AT+ZLOCKCELL?";
    let active = "LTE,earfcn:1300,pci:3,status:on\nNR5G,nr5g_chanel:633984,pci:0,scs:1,band:78,status:on\nOK\n";
    let partial = "LTE,earfcn:1300,pci:3,status:on\nNR5G,status:off\nOK\n";
    let mut frames = replies("gm800");
    frames.insert(query.into(), active.into());
    let device = Arc::new(TypedDevice {
        model: "gm800",
        shared: true,
        replies: Mutex::new(frames),
        writes: BTreeMap::from([("AT+ZLOCKCELL=0".into(), (query.into(), partial.into()))]),
        calls: Mutex::new(vec![]),
    });
    let control = controller(&device);
    let op = json!({"rat":"NR","mode":"auto","entries":[]});
    let p = control.prepare("m1", &op, "owner").unwrap();
    device
        .replies
        .lock()
        .unwrap()
        .insert(query.into(), active.replace("pci:3", "pci:4"));
    assert_eq!(execute(&control, &op, &p).unwrap_err().code, "LOCK_CHANGED");
    assert!(
        !device
            .calls
            .lock()
            .unwrap()
            .contains(&"AT+ZLOCKCELL=0".into())
    );
    let p = control.prepare("m1", &op, "owner").unwrap();
    let result = execute(&control, &op, &p).unwrap();
    assert_eq!(
        result["savedVerified"], false,
        "NR unlocked alone is not a successful global unlock"
    );
}
#[test]
fn band_transition_checks_other_rat_preservation_and_global_auto_scope() {
    let before = bands::parse("rm520n-gl", &replies("rm520n-gl")).unwrap();
    let mut after = before.clone();
    after["groups"]["NR"]["selectedBands"] = json!([78]);
    assert!(bands::matched_transition(
        "rm520n-gl",
        &before,
        &after,
        &op("NR", 78)
    ));
    after["groups"]["LTE"]["selectedBands"] = json!([3]);
    assert!(!bands::matched_transition(
        "rm520n-gl",
        &before,
        &after,
        &op("NR", 78)
    ));
    let before = bands::parse("gm800", &replies("gm800")).unwrap();
    let mut after = before.clone();
    after["groups"]["NR"]["selectedBands"] = json!([41, 78, 79]);
    let reset = json!({"rat":"NR","mode":"auto","target":"band","entries":[]});
    assert!(!bands::matched_transition("gm800", &before, &after, &reset));
    after["groups"]["LTE"]["selectedBands"] = json!([1, 3, 5]);
    assert!(bands::matched_transition("gm800", &before, &after, &reset));
}
#[test]
fn cell_write_requires_live_or_exact_supported_inventory_before_confirmation() {
    let mut frames = replies("fm350-gl");
    frames.remove("AT+GTACT=?");
    frames.insert("AT+GTCELLLOCK?".into(), "+GTCELLLOCK:0\nOK\n".into());
    let device = Arc::new(TypedDevice {
        model: "fm350-gl",
        shared: true,
        replies: Mutex::new(frames),
        writes: BTreeMap::new(),
        calls: Mutex::new(vec![]),
    });
    let control = controller(&device);
    let op = json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":0,"scsType":1}]});
    assert_eq!(
        control.prepare("m1", &op, "owner").unwrap_err().code,
        "LOCK_BAND_INVENTORY_REQUIRED"
    );
    assert!(
        device
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| !c.starts_with("AT+GTCELLLOCK="))
    );
}

#[test]
fn fibocom_single_global_cell_slot_binds_previous_rat_when_replacing() {
    let query = "AT+GTCELLLOCK?";
    let mut frames = replies("fm350-gl");
    frames.insert(query.into(), "+GTCELLLOCK:1,0,0,1300,3\nOK\n".into());
    let device = Arc::new(TypedDevice {
        model: "fm350-gl",
        shared: false,
        replies: Mutex::new(frames),
        writes: BTreeMap::new(),
        calls: Mutex::new(vec![]),
    });
    let control = controller(&device);
    let op = json!({"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":633984,"pci":0,"scsType":1}]});
    let p = control.prepare("m1", &op, "owner").unwrap();
    assert!(p["warning"].as_str().unwrap().contains("替换"));
    device
        .replies
        .lock()
        .unwrap()
        .insert(query.into(), "+GTCELLLOCK:1,0,0,1300,4\nOK\n".into());
    assert_eq!(execute(&control, &op, &p).unwrap_err().code, "LOCK_CHANGED");
    assert!(
        device
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| !c.starts_with("AT+GTCELLLOCK="))
    );
    assert_eq!(control.read("m1", true).unwrap()["cellWriteScope"], "modem");
}
