#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use wlmb_server::{
    PanelError, Result,
    actions::Actions,
    events::{Events, wall_ms},
    sampler::{Sampler, Source},
    sampling::{self, Sample},
};
#[test]
fn transient_busy_keeps_only_unexpired_primary_measurements_without_renewing_the_clock() {
    let row = modem("m1");
    let commands = vec!["AT^MONSC".to_owned()];
    let samples = BTreeMap::from([(
        commands[0].clone(),
        Sample {
            raw: MONSC.into(),
            wall_ms: 1000,
            mono: Instant::now(),
        },
    )]);
    for code in [
        "AT_BUSY",
        "SYSTEM_BUSY",
        "AT_COOLDOWN",
        "AT_PORT_CONFLICT",
        "AT_OWNERSHIP_UNKNOWN",
    ] {
        let errors = vec![json!({"command":commands[0],"code":code})];
        let fresh = sampling::publish(&row, &commands, &samples, &errors, 5000).unwrap();
        assert_eq!(fresh["stale"], false);
        assert_eq!(fresh["refreshDeferred"], true);
        assert_eq!(fresh["observedAtMs"], 1000);
        assert_eq!(fresh["partial"], true);
        assert!(!fresh["serving"].as_array().unwrap().is_empty());
        assert_eq!(
            sampling::publish(&row, &commands, &samples, &errors, 16001).unwrap()["stale"],
            true
        );
        assert_eq!(
            sampling::publish(&row, &commands, &samples, &errors, 999).unwrap()["stale"],
            true
        );
    }
    for code in [
        "MODEM_CHANGED",
        "AT_PARSE_UNAVAILABLE",
        "AT_UNSUPPORTED",
        "AT_CHANNEL_QUARANTINED",
    ] {
        assert_eq!(
            sampling::publish(
                &row,
                &commands,
                &samples,
                &[json!({"command":commands[0],"code":code})],
                5000
            )
            .unwrap()["stale"],
            true
        );
    }
    assert_eq!(
        sampling::publish(
            &row,
            &commands,
            &BTreeMap::new(),
            &[json!({"command":commands[0],"code":"AT_BUSY"})],
            5000
        )
        .unwrap()["stale"],
        true
    );
}

#[test]
fn temperature_has_its_own_age_and_rejects_clock_rollback() {
    let mut snapshot =
        json!({"observedAtMs":100001,"temperature":{"celsius":47,"observedAtMs":10000}});
    sampling::age_temperature(&mut snapshot, 100000);
    assert_eq!(snapshot["temperature"]["stale"], false);
    sampling::age_temperature(&mut snapshot, 100001);
    assert_eq!(snapshot["temperature"]["stale"], true);
    for sensor in [
        json!({"celsius":47}),
        json!({"celsius":47,"observedAtMs":100002}),
    ] {
        let mut snapshot = json!({"temperature":sensor});
        sampling::age_temperature(&mut snapshot, 100001);
        assert_eq!(snapshot["temperature"]["stale"], true);
    }
}

#[test]
fn temperature_fallback_prefers_fresh_valid_sensor_and_keeps_its_own_timestamp() {
    let model = "fm160-cn";
    let row = json!({"id":"m1","model":model,"profile":wlmb_server::catalog::identify(model)});
    let main = "+GTCCINFO:\n1,9,460,01,ABC,12345,992F0,1E8,5078,12,79,0,76,69\nOK\n";
    let commands: Vec<_> = ["AT+GTCCINFO?", "AT+MTSM=1,6", "AT+GTLADC"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut samples = BTreeMap::from([
        (
            commands[0].clone(),
            Sample {
                raw: main.into(),
                wall_ms: 100000,
                mono: Instant::now(),
            },
        ),
        (
            commands[1].clone(),
            Sample {
                raw: "+MTSM: 40\nOK\n".into(),
                wall_ms: 30000,
                mono: Instant::now(),
            },
        ),
        (
            commands[2].clone(),
            Sample {
                raw: "+GTLADC: cpu 45000\nOK\n".into(),
                wall_ms: 99000,
                mono: Instant::now(),
            },
        ),
    ]);
    let first = sampling::publish(&row, &commands, &samples, &[], 100000).unwrap();
    assert_eq!(first["temperature"]["celsius"], 45.);
    assert_eq!(first["temperature"]["observedAtMs"], 99000);
    samples.get_mut(&commands[2]).unwrap().wall_ms = 100001;
    assert_eq!(
        sampling::publish(&row, &commands, &samples, &[], 100000).unwrap()["temperature"]["celsius"],
        40.
    );
    samples.get_mut(&commands[1]).unwrap().raw = "+MTSM: 40\nERROR\n".into();
    assert!(
        sampling::publish(&row, &commands, &samples, &[], 100000).unwrap()["temperature"].is_null()
    );
}

#[test]
fn healthy_temperature_skips_fallback_and_unsupported_temperature_uses_it_once() {
    for first_supported in [true, false] {
        let fake = Fake::new();
        let mut row = modem("m1");
        row["model"] = json!("fm160-cn");
        row["profile"] = wlmb_server::catalog::identify("fm160-cn");
        *fake.rows.lock().unwrap() = vec![row];
        {
            let mut replies = fake.responses.lock().unwrap();
            replies.insert(
                "AT+GTCCINFO?".into(),
                Ok("+GTCCINFO:\n1,9,460,01,ABC,12345,992F0,1E8,5078,12,79,0,76,69\nOK\n".into()),
            );
            if first_supported {
                replies.insert("AT+MTSM=1,6".into(), Ok("+MTSM: 40\nOK\n".into()));
            }
            replies.insert("AT+GTLADC".into(), Ok("+GTLADC: cpu 45000\nOK\n".into()));
        }
        let panel = Sampler::new(fake.clone());
        panel.tick().unwrap();
        wait(|| {
            fake.count("AT+GTCCINFO?") == 1
                && panel.status(None).unwrap()["radio"]["pending"] == false
        });
        assert_eq!(fake.count("AT+MTSM=1,6"), 1);
        assert_eq!(fake.count("AT+GTLADC"), usize::from(!first_supported));
        assert_eq!(
            panel.status(None).unwrap()["radio"]["temperature"]["celsius"],
            if first_supported { 40. } else { 45. }
        );
        assert!(panel.stop(Duration::from_secs(1)).unwrap());
    }
}

#[test]
fn removed_or_unknown_selected_modem_never_falls_back_to_another_device() {
    let fake = Fake::new();
    let panel = Sampler::new(fake.clone());
    panel.discover().unwrap();
    assert_eq!(panel.status(None).unwrap()["selectedModem"], "m1");
    assert_eq!(
        panel.status(Some("missing")).unwrap_err().code,
        "MODEM_NOT_READY"
    );
    fake.rows.lock().unwrap().clear();
    panel.discover().unwrap();
    assert!(panel.status(None).unwrap()["selectedModem"].is_null());
    assert_eq!(
        panel.status(Some("m1")).unwrap_err().code,
        "MODEM_NOT_READY"
    );
}

mod typed_controls {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use wlmb_server::{radio_controls::RadioControls, security::Security};
    struct Modem {
        nr: Mutex<String>,
        readback: Mutex<Option<String>>,
        calls: Mutex<Vec<String>>,
        result: Result<String>,
    }
    impl Source for Modem {
        fn modems(&self) -> Result<Vec<Value>> {
            Ok(vec![modem("m1")])
        }
        fn query(&self, _id: &str, command: &str, _port: &str, _model: &str) -> Result<String> {
            self.calls.lock().unwrap().push(command.into());
            match command {
                "AT^NRFREQLOCK?" => Ok(self.nr.lock().unwrap().clone()),
                "AT^LTEFREQLOCK?" => Ok("^LTEFREQLOCK: 0\nOK\n".into()),
                "AT^NRFREQLOCK=3,1,1,\"78\"" => {
                    *self.nr.lock().unwrap() = self
                        .readback
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(|| "^NRFREQLOCK: 3\n1,1\n78\nOK\n".into());
                    self.result.clone()
                }
                _ => Err(PanelError::new("UNEXPECTED_TEST_COMMAND", 422)),
            }
        }
    }
    fn setup(result: Result<String>) -> (Arc<Modem>, RadioControls, Arc<AtomicBool>) {
        let source = Arc::new(Modem {
            nr: Mutex::new("^NRFREQLOCK: 0\nOK\n".into()),
            readback: Mutex::new(None),
            calls: Mutex::new(vec![]),
            result,
        });
        let enabled = Arc::new(AtomicBool::new(false));
        let controls = RadioControls::new(
            source.clone(),
            Arc::new(Mutex::new(Security::default())),
            enabled.clone(),
            Instant::now(),
        );
        (source, controls, enabled)
    }
    fn operation() -> Value {
        json!({"rat":"NR","mode":"band","entries":[{"band":78}]})
    }
    fn execute(controls: &RadioControls, prepared: &Value, owner: &str) -> Result<Value> {
        controls.execute(
            "m1",
            &operation(),
            prepared["revision"].as_str().unwrap(),
            prepared["confirmation"].as_str().unwrap(),
            owner,
        )
    }
    #[test]
    fn disabled_typed_write_does_not_query_and_cached_state_does_not_query() {
        let (source, controls, _enabled) = setup(Ok("OK".into()));
        assert_eq!(
            controls
                .prepare("m1", &operation(), "owner")
                .unwrap_err()
                .code,
            "AT_WRITE_DISABLED"
        );
        assert!(source.calls.lock().unwrap().is_empty());
        let value = controls.read("m1", false).unwrap();
        assert_eq!(value["groups"]["NR"]["mode"], 0);
        controls.read("m1", false).unwrap();
        assert!(controls.state("m1", &[modem("m1")]).is_some());
        assert_eq!(source.calls.lock().unwrap().len(), 2);
        let mut new = modem("m1");
        new["port"] = json!("/dev/ttyUSB9");
        assert!(controls.state("m1", &[new]).is_none());
    }
    #[test]
    fn confirmation_is_owner_bound_consumed_and_cannot_be_reused() {
        let (source, controls, enabled) = setup(Ok("OK".into()));
        enabled.store(true, Ordering::Release);
        let prepared = controls.prepare("m1", &operation(), "owner").unwrap();
        assert!(execute(&controls, &prepared, "other").is_err());
        assert!(execute(&controls, &prepared, "owner").is_err());
        assert_eq!(source.calls.lock().unwrap().len(), 2);
    }
    #[test]
    fn restoring_mt_band_preferences_cannot_silently_unlock_its_shared_cell_slot() {
        let (source, controls, enabled) = setup(Ok("OK".into()));
        enabled.store(true, Ordering::Release);
        *source.nr.lock().unwrap() = "^NRFREQLOCK: 2\n0,1\n78,633984,1,505\nOK\n".into();
        let reset = json!({"rat":"NR","mode":"auto","entries":[],"target":"band"});
        assert_eq!(
            controls.prepare("m1", &reset, "owner").unwrap_err().code,
            "LOCK_BAND_CELL_CONFLICT"
        );
        assert!(
            source
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|c| c.ends_with('?'))
        );
        let prepared = controls.prepare("m1", &operation(), "owner").unwrap();
        assert!(prepared["warning"].as_str().unwrap().contains("替换"));
        assert_eq!(
            controls.read("m1", true).unwrap()["bandCellExclusive"],
            true
        );
    }
    #[test]
    fn changed_saved_config_blocks_write_after_consuming_confirmation() {
        let (source, controls, enabled) = setup(Ok("OK".into()));
        enabled.store(true, Ordering::Release);
        let prepared = controls.prepare("m1", &operation(), "owner").unwrap();
        *source.nr.lock().unwrap() = "^NRFREQLOCK: 3\n1,1\n41\nOK\n".into();
        assert_eq!(
            execute(&controls, &prepared, "owner").unwrap_err().code,
            "LOCK_CHANGED"
        );
        assert!(execute(&controls, &prepared, "owner").is_err());
        assert!(
            source
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|c| c.ends_with('?'))
        );
    }
    #[test]
    fn uncertain_save_is_never_replayed_even_if_device_actually_saved() {
        let (source, controls, enabled) = setup(Err(PanelError::new("SYSTEM_UNAVAILABLE", 503)));
        enabled.store(true, Ordering::Release);
        let prepared = controls.prepare("m1", &operation(), "owner").unwrap();
        assert_eq!(
            execute(&controls, &prepared, "owner").unwrap_err().code,
            "LOCK_RESULT_UNKNOWN"
        );
        assert!(execute(&controls, &prepared, "owner").is_err());
        assert_eq!(
            source
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.contains("=3"))
                .count(),
            1
        );
        assert!(controls.state("m1", &[modem("m1")]).is_none());
        assert_eq!(
            controls.read("m1", true).unwrap()["groups"]["NR"]["mode"],
            3
        );
    }
    #[test]
    fn save_and_readback_do_not_claim_activation_or_reset_radio() {
        let (source, controls, enabled) = setup(Ok("OK\n".into()));
        enabled.store(true, Ordering::Release);
        let prepared = controls.prepare("m1", &operation(), "owner").unwrap();
        let result = execute(&controls, &prepared, "owner").unwrap();
        assert!(
            prepared["acceptedAtMs"].is_null(),
            "Preparation is not a save receipt"
        );
        assert!(
            result["acceptedAtMs"]
                .as_u64()
                .is_some_and(|at| at <= wall_ms())
        );
        assert_eq!(result["savedVerified"], true);
        assert_eq!(result["activation"], "unverified");
        assert_eq!(result["requiresExternalActivation"], true);
        assert_eq!(
            *source.calls.lock().unwrap(),
            [
                "AT^NRFREQLOCK?",
                "AT^LTEFREQLOCK?",
                "AT^NRFREQLOCK?",
                "AT^LTEFREQLOCK?",
                "AT^NRFREQLOCK=3,1,1,\"78\"",
                "AT^NRFREQLOCK?",
                "AT^LTEFREQLOCK?"
            ]
        );
    }
    #[test]
    fn contradictory_write_ack_is_unknown_and_cannot_be_replayed() {
        for raw in ["OK\nERROR", "OK\nOK", "OK\nother"] {
            let (source, controls, enabled) = setup(Ok(raw.into()));
            enabled.store(true, Ordering::Release);
            let prepared = controls.prepare("m1", &operation(), "owner").unwrap();
            assert_eq!(
                execute(&controls, &prepared, "owner").unwrap_err().code,
                "LOCK_RESULT_UNKNOWN"
            );
            assert!(execute(&controls, &prepared, "owner").is_err());
            assert_eq!(
                source
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| c.contains("=3"))
                    .count(),
                1
            );
            assert!(controls.state("m1", &[modem("m1")]).is_none());
        }
    }
    #[test]
    fn exclusion_list_is_not_verified_as_the_requested_band_allowlist() {
        let (source, controls, enabled) = setup(Ok("OK".into()));
        enabled.store(true, Ordering::Release);
        *source.readback.lock().unwrap() = Some("^NRFREQLOCK: 3\n0,1\n78\nOK\n".into());
        let prepared = controls.prepare("m1", &operation(), "owner").unwrap();
        assert_eq!(
            execute(&controls, &prepared, "owner").unwrap()["savedVerified"],
            false
        );
    }
}

fn wait(mut check: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(Instant::now() < until, "bounded test wait expired");
        thread::sleep(Duration::from_millis(5));
    }
}
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn hold(&self) {
        let open = self.open.lock().unwrap();
        let (open, result) = self
            .changed
            .wait_timeout_while(open, Duration::from_secs(3), |v| !*v)
            .unwrap();
        assert!(*open && !result.timed_out());
    }
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
fn modem(key: &str) -> Value {
    json!({"id":key,"port":"/dev/ttyUSB1","model":"mt5700m-cn","shared":true,"profile":wlmb_server::catalog::identify("mt5700m-cn")})
}
const MONSC: &str = "^MONSC: NR,460,01,627264,1,ABC,1E8,ABCD,-81,-5,16\n";
const HFREQ: &str = "^HFREQINFO: 0,7,78,630000,3450000,100000,630000,3450000,100000,78,636664,3549960,100000,636664,3549960,100000\n";
#[test]
fn sierra_lte_only_registration_survives_empty_or_unsupported_nr_in_both_modes() {
    const LTE: &str = "!LTEINFO:\nServing: EARFCN MCC MNC TAC CID Bd D U SNR PCI RSRQ RSRP RSSI RXLV\n1800 460 01 42 00FF0012 3 5 4 12 90 -8 -830 -66 40\nIntraFreq: PCI RSRQ RSRP RSSI RXLV\n91 -10 -900 -70 20\nOK\n";
    for model in ["EM9190", "EM9291"] {
        for shared in [false, true] {
            for first in [
                Ok("OK\n".to_owned()),
                Err(PanelError::new("AT_UNSUPPORTED", 422)),
            ] {
                let fake = Fake::new();
                *fake.rows.lock().unwrap() = vec![
                    json!({"id":"m1","model":model,"port":"/dev/ttyUSB1","shared":shared,"transport":if shared {"shared"}else{"dedicated"},"channelReady":true,"identityVerified":true,"profile":wlmb_server::catalog::identify(model)}),
                ];
                fake.responses
                    .lock()
                    .unwrap()
                    .insert("AT!NRINFO?".into(), first);
                fake.responses
                    .lock()
                    .unwrap()
                    .insert("AT!LTEINFO?".into(), Ok(LTE.into()));
                let panel = Sampler::new(fake.clone());
                panel.tick().unwrap();
                wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
                let value = panel.status(None).unwrap();
                assert_eq!(value["radio"]["stale"], false);
                assert_eq!(value["radio"]["serving"][0]["rat"], "LTE");
                assert_eq!(value["radio"]["serving"][0]["rsrp"], -83.);
                assert_eq!(value["radio"]["neighbors"][0]["pci"], 91.);
                assert_eq!(fake.count("AT!LTEINFO?"), 1);
                assert!(
                    fake.calls
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|(_, cmd)| !cmd.contains("CFUN"))
                );
                assert!(panel.stop(Duration::from_secs(1)).unwrap());
            }
        }
    }
}
#[test]
fn nsa_signal_only_measurement_has_its_own_age_without_becoming_a_carrier() {
    let mut fields = vec!["0"; 35];
    for (index, value) in [
        (0, "LTE-NR"),
        (2, "460"),
        (3, "01"),
        (5, "90"),
        (7, "00FF0012"),
        (8, "42"),
        (9, "1800"),
        (10, "100"),
        (14, "-80"),
        (15, "-10"),
        (16, "100"),
        (29, "-85"),
        (30, "-10"),
        (31, "150"),
        (32, "78"),
        (34, "100"),
    ] {
        fields[index] = value;
    }
    let commands = vec!["AT^CELLINFO=1".to_owned()];
    let row = json!({"profile":wlmb_server::catalog::identify("SRM815")});
    let samples = BTreeMap::from([(
        commands[0].clone(),
        Sample {
            raw: format!("^CELLINFO:{}\nOK\n", fields.join(",")),
            wall_ms: 1000,
            mono: Instant::now(),
        },
    )]);
    let fresh = sampling::publish(&row, &commands, &samples, &[], 1001).unwrap();
    assert_eq!(fresh["secondaryMeasurement"]["stale"], false);
    assert_eq!(fresh["secondaryMeasurement"]["observedAtMs"], 1000);
    assert!(fresh["carriers"].as_array().unwrap().is_empty());
    let old = sampling::publish(&row, &commands, &samples, &[], 16001).unwrap();
    assert_eq!(old["secondaryMeasurement"]["stale"], true);
}
struct Fake {
    rows: Mutex<Vec<Value>>,
    calls: Mutex<Vec<(String, String)>>,
    responses: Mutex<HashMap<String, Result<String>>>,
    gates: Mutex<HashMap<String, Arc<Gate>>>,
    discoveries: AtomicUsize,
    discovery_error: AtomicBool,
}
impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            rows: Mutex::new(vec![modem("m1")]),
            calls: Mutex::new(vec![]),
            responses: Mutex::new(HashMap::from([
                ("AT^MONSC".into(), Ok(MONSC.into())),
                ("AT^HFREQINFO?".into(), Ok(HFREQ.into())),
                ("AT^MONNC".into(), Ok(String::new())),
            ])),
            gates: Mutex::new(HashMap::new()),
            discoveries: AtomicUsize::new(0),
            discovery_error: AtomicBool::new(false),
        })
    }
    fn gate(&self, key: &str) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        self.gates.lock().unwrap().insert(key.into(), gate.clone());
        gate
    }
    fn count(&self, key: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, c)| c == key)
            .count()
    }
}
impl Source for Fake {
    fn modems(&self) -> Result<Vec<Value>> {
        self.discoveries.fetch_add(1, Ordering::SeqCst);
        if self.discovery_error.load(Ordering::Acquire) {
            return Err(PanelError::new("SYSTEM_UNAVAILABLE", 503));
        }
        Ok(self.rows.lock().unwrap().clone())
    }
    fn query(&self, key: &str, command: &str, _port: &str, _model: &str) -> Result<String> {
        let answer = self
            .responses
            .lock()
            .unwrap()
            .get(command)
            .cloned()
            .unwrap_or(Err(PanelError::new("AT_UNSUPPORTED", 422)));
        let gate = self.gates.lock().unwrap().get(command).cloned();
        self.calls
            .lock()
            .unwrap()
            .push((key.into(), command.into()));
        if let Some(gate) = gate {
            gate.hold();
        }
        answer
    }
}

#[test]
fn receipts_are_owner_bound_bounded_and_never_replay() {
    let actions = Actions::default();
    let gate = Arc::new(Gate::default());
    let count = Arc::new(AtomicUsize::new(0));
    let submit = || {
        let gate = gate.clone();
        let count = count.clone();
        actions
            .submit("owner", move || {
                count.fetch_add(1, Ordering::SeqCst);
                gate.hold();
                Ok(json!({"verified":true}))
            })
            .unwrap()
    };
    let a = submit();
    let b = submit();
    wait(|| count.load(Ordering::SeqCst) == 2);
    assert_eq!(
        actions.submit("owner", || Ok(json!({}))).unwrap_err().code,
        "CONTROL_BUSY"
    );
    let key = a["job"].as_str().unwrap();
    assert!(actions.result("other", key).is_err());
    for _ in 0..50 {
        assert_eq!(actions.result("owner", key).unwrap()["state"], "pending");
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
    gate.release();
    wait(|| actions.result("owner", key).unwrap()["state"] == "done");
    wait(|| actions.result("owner", b["job"].as_str().unwrap()).unwrap()["state"] == "done");
    assert_eq!(
        actions.result("owner", key).unwrap()["result"]["verified"],
        true
    );
    for _ in 0..10 {
        actions.result("owner", key).unwrap();
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert!(actions.submit("", || Ok(json!({}))).is_err());
}
#[test]
fn stopping_drains_accepted_work_and_rejects_new_actions() {
    let actions = Actions::default();
    let gate = Arc::new(Gate::default());
    let worker_gate = gate.clone();
    let accepted = actions
        .submit("owner", move || {
            worker_gate.hold();
            Ok(json!({"saved":true}))
        })
        .unwrap();
    assert!(!actions.shutdown(Duration::from_millis(10)));
    assert_eq!(
        actions.submit("owner", || Ok(json!({}))).unwrap_err().code,
        "SERVICE_STOPPING"
    );
    gate.release();
    assert!(actions.shutdown(Duration::from_secs(2)));
    assert_eq!(
        actions
            .result("owner", accepted["job"].as_str().unwrap())
            .unwrap()["state"],
        "done"
    );
}
#[test]
fn event_log_is_redacted_rate_limited_and_bounded() {
    let events = Events::default();
    events.record("badsource", "PRIVATE");
    assert_eq!(events.recent(), json!([]));
    for _ in 0..20 {
        events.record("sampler", "password synthetic-secret");
    }
    let text = events.recent().to_string();
    assert!(!text.contains("secret"));
    assert_eq!(events.recent().as_array().unwrap().len(), 1);
    assert!(text.contains("UNCLASSIFIED_ERROR"));
    for i in 0..80 {
        events.record("sampler", &format!("TEST_{i}"));
    }
    assert_eq!(events.recent().as_array().unwrap().len(), 32);
}
#[test]
fn snapshot_aggregate_bound_is_distinct_from_one_response_bound() {
    let now = wall_ms();
    let sample = |raw: String| Sample {
        raw,
        wall_ms: now,
        mono: Instant::now(),
    };
    let commands: Vec<String> = vec!["AT^MONSC".into(), "AT^HFREQINFO?".into()];
    let samples = BTreeMap::from([
        (
            commands[0].clone(),
            sample(format!("{MONSC}{}", " ".repeat(40000))),
        ),
        (
            commands[1].clone(),
            sample(format!("{HFREQ}{}", " ".repeat(40000))),
        ),
    ]);
    let data = sampling::publish(&modem("m1"), &commands, &samples, &[], now).unwrap();
    assert_eq!(data["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(data["serving"][0]["rsrp"], -81.0);
    let mut bad = samples;
    bad.get_mut(&commands[0]).unwrap().raw = "x".repeat(65537);
    assert!(sampling::publish(&modem("m1"), &commands, &bad, &[], now).is_err());
}
#[test]
fn old_metric_intervals_are_removed_without_fabricating_values() {
    let mut row = json!({"rsrp":-81,"rssi":-60,"metricObservedAtMs":{"rsrp":20000,"rssi":null},"metricIntervals":{"rsrp":{"low":-82},"rssi":{"low":-61}}});
    sampling::age_metrics(&mut row, 35001);
    assert!(row["rsrp"].is_null());
    assert!(row["rssi"].is_null());
    assert_eq!(row["metricIntervals"], json!({}));
}
#[test]
fn subscription_qci_and_ims_have_independent_expiry_and_source_stamps() {
    let now = wall_ms();
    let commands: Vec<String> = ["AT^MONSC", "AT^DSAMBR=1", "AT+CGEQOSRDP", "AT^IMSSWITCH?"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut samples = BTreeMap::new();
    for (cmd, raw, age) in [
        ("AT^MONSC", MONSC, 0),
        (
            "AT^DSAMBR=1",
            "^DSAMBR: 1,2000000,200000,\"internet\",5\n",
            60000,
        ),
        ("AT+CGEQOSRDP", "+CGEQOSRDP: 1,6\n", 60000),
        ("AT^IMSSWITCH?", "^IMSSWITCH: 0,0,0\n", 60000),
    ] {
        samples.insert(
            cmd.into(),
            Sample {
                raw: raw.into(),
                wall_ms: now - age,
                mono: Instant::now(),
            },
        );
    }
    let value = sampling::publish(&modem("m1"), &commands, &samples, &[], now).unwrap();
    assert_eq!(value["subscription"]["observedAtMs"], now - 60000);
    assert_eq!(value["sessionQos"]["qci"], 6);
    assert_eq!(value["smsReceiving"]["stale"], false);
    for key in commands.iter().skip(1) {
        samples.get_mut(key).unwrap().wall_ms = now - 90001;
    }
    let value = sampling::publish(&modem("m1"), &commands, &samples, &[], now).unwrap();
    assert!(value["subscription"].is_null());
    assert!(value["sessionQos"].is_null());
    assert!(value["smsReceiving"].is_null());
}
#[test]
fn five_bounded_replies_and_future_samples_are_handled_consistently() {
    let now = wall_ms();
    let commands: Vec<String> = [
        "AT^MONSC",
        "AT^HFREQINFO?",
        "AT^MONNC",
        "AT^DSAMBR=1",
        "AT^CHIPTEMP?",
    ]
    .iter()
    .map(|s| (*s).into())
    .collect();
    let mut samples = BTreeMap::new();
    for (i, command) in commands.iter().enumerate() {
        let mut raw = if i == 0 {
            MONSC.to_owned()
        } else {
            String::new()
        };
        raw.push_str(&" ".repeat(wlmb_server::MAX_AT_RESPONSE - raw.len()));
        samples.insert(
            command.clone(),
            Sample {
                raw,
                wall_ms: now,
                mono: Instant::now(),
            },
        );
    }
    let data = sampling::publish(&modem("m1"), &commands, &samples, &[], now).unwrap();
    assert_eq!(data["serving"][0]["rsrp"], -81.0);
    samples.get_mut(&commands[0]).unwrap().wall_ms = now + 1;
    let future = sampling::publish(&modem("m1"), &commands, &samples, &[], now).unwrap();
    assert!(
        future["serving"].as_array().unwrap().is_empty(),
        "Clock rollback must not publish future signal observations"
    );
    let mut row = json!({"rsrp":-80,"metricObservedAtMs":{"rsrp":now+1},"metricIntervals":{"rsrp":{"low":-81}}});
    sampling::age_metrics(&mut row, now);
    assert!(row["rsrp"].is_null());
    assert_eq!(row["metricIntervals"], json!({}));
}
#[test]
fn first_signal_publishes_before_optional_and_readers_never_sample() {
    let fake = Fake::new();
    let gate = fake.gate("AT^HFREQINFO?");
    let panel = Sampler::new(fake.clone());
    for _ in 0..100 {
        panel.status(None).unwrap();
    }
    assert_eq!(fake.discoveries.load(Ordering::SeqCst), 0);
    panel.tick().unwrap();
    wait(|| fake.count("AT^HFREQINFO?") == 1);
    for _ in 0..100 {
        let data = panel.status(None).unwrap();
        assert_eq!(data["radio"]["serving"][0]["rsrp"], -81.0);
        assert_eq!(data["radio"]["pending"], true);
    }
    assert_eq!(fake.count("AT^MONSC"), 1);
    assert_eq!(fake.discoveries.load(Ordering::SeqCst), 1);
    assert_eq!(panel.refresh("m1").unwrap()["accepted"], false);
    gate.release();
    wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
    assert_eq!(
        panel.status(None).unwrap()["radio"]["carriers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        fake.count("AT^DSAMBR=1"),
        1,
        "SA must query subscription AMBR too"
    );
    assert!(panel.stop(Duration::from_secs(1)).unwrap());
}
#[test]
fn optional_failure_stops_sequence_and_keeps_real_primary() {
    let fake = Fake::new();
    fake.responses.lock().unwrap().insert(
        "AT^HFREQINFO?".into(),
        Err(PanelError::new("AT_FAILED", 503)),
    );
    let panel = Sampler::new(fake.clone());
    panel.tick().unwrap();
    wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
    let data = panel.status(None).unwrap();
    assert_eq!(data["radio"]["serving"][0]["rsrp"], -81.0);
    assert_eq!(data["radio"]["partial"], true);
    assert_eq!(fake.count("AT^MONNC"), 0);
    assert_eq!(fake.count("AT^HFREQINFO?"), 1);
    panel.stop(Duration::from_secs(1)).unwrap();
}
#[test]
fn late_hotplug_worker_cannot_publish_into_replacement() {
    let fake = Fake::new();
    let gate = fake.gate("AT^MONSC");
    let panel = Sampler::new(fake.clone());
    panel.tick().unwrap();
    wait(|| fake.count("AT^MONSC") == 1);
    fake.rows.lock().unwrap().clear();
    panel.discover().unwrap();
    let mut new = modem("m1");
    new["port"] = json!("/dev/ttyUSB9");
    fake.rows.lock().unwrap().push(new);
    panel.discover().unwrap();
    assert_eq!(panel.status(None).unwrap()["radio"]["serving"], json!([]));
    gate.release();
    assert!(panel.stop(Duration::from_secs(1)).unwrap());
    assert_eq!(panel.status(None).unwrap()["radio"]["serving"], json!([]));
    assert_eq!(fake.count("AT^HFREQINFO?"), 0);
}
#[test]
fn four_worker_budget_survives_hotplug_and_shutdown_does_not_send_more() {
    let fake = Fake::new();
    *fake.rows.lock().unwrap() = (0..8).map(|i| modem(&format!("m{i}"))).collect();
    let gate = fake.gate("AT^MONSC");
    let panel = Sampler::new(fake.clone());
    panel.tick().unwrap();
    wait(|| fake.count("AT^MONSC") == 4);
    fake.rows.lock().unwrap().clear();
    panel.discover().unwrap();
    assert!(!panel.stop(Duration::from_millis(10)).unwrap());
    gate.release();
    assert!(panel.stop(Duration::from_secs(1)).unwrap());
    assert_eq!(fake.count("AT^MONSC"), 4);
    assert_eq!(fake.count("AT^HFREQINFO?"), 0);
}
#[test]
fn periodic_poller_does_not_spin_after_worker_completion() {
    let fake = Fake::new();
    let panel = Sampler::new(fake.clone());
    panel.set_interval(2).unwrap();
    let other = panel.clone();
    let poll = thread::spawn(move || other.run().unwrap());
    wait(|| fake.count("AT^MONNC") == 1);
    thread::sleep(Duration::from_millis(120));
    assert_eq!(fake.discoveries.load(Ordering::SeqCst), 1);
    assert!(panel.stop(Duration::from_secs(1)).unwrap());
    poll.join().unwrap();
    assert!(panel.set_interval(1).is_err());
}

#[test]
fn cell_change_invalidates_prior_ca_before_slow_optional_reply() {
    let fake = Fake::new();
    let panel = Sampler::new(fake.clone());
    panel.tick().unwrap();
    wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
    assert_eq!(
        panel.status(None).unwrap()["radio"]["carriers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fake.responses
        .lock()
        .unwrap()
        .insert("AT^MONSC".into(), Ok(MONSC.replace("1E8", "1F4")));
    let gate = fake.gate("AT^HFREQINFO?");
    thread::sleep(Duration::from_millis(810));
    assert_eq!(panel.refresh("m1").unwrap()["accepted"], true);
    wait(|| fake.count("AT^HFREQINFO?") == 2);
    let data = panel.status(None).unwrap();
    assert_eq!(data["radio"]["serving"][0]["pci"].as_f64(), Some(500.0));
    assert!(
        data["radio"]["carriers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["channel"].as_f64() != Some(630000.0)
                && c["channel"].as_f64() != Some(636664.0))
    );
    gate.release();
    wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
    panel.stop(Duration::from_secs(1)).unwrap();
}
#[test]
fn ok_only_primary_is_not_published_as_a_fresh_measurement() {
    let fake = Fake::new();
    let panel = Sampler::new(fake.clone());
    panel.tick().unwrap();
    wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
    let before = panel.status(None).unwrap()["radio"]["observedAtMs"].clone();
    fake.responses
        .lock()
        .unwrap()
        .insert("AT^MONSC".into(), Ok("OK\n".into()));
    thread::sleep(Duration::from_millis(810));
    panel.refresh("m1").unwrap();
    wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
    let data = panel.status(None).unwrap();
    assert_eq!(data["radio"]["observedAtMs"], before);
    assert_eq!(data["radio"]["stale"], true);
    assert_eq!(data["radio"]["partial"], true);
    assert_eq!(fake.count("AT^HFREQINFO?"), 1);
    assert_eq!(data["radio"]["serving"][0]["rsrp"], -81.0);
    panel.stop(Duration::from_secs(1)).unwrap();
}
#[test]
fn unsupported_optional_is_cooled_down_but_still_reported_partial() {
    let fake = Fake::new();
    fake.responses.lock().unwrap().insert(
        "AT^MONNC".into(),
        Err(PanelError::new("AT_UNSUPPORTED", 422)),
    );
    let panel = Sampler::new(fake.clone());
    panel.tick().unwrap();
    wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
    thread::sleep(Duration::from_millis(810));
    panel.refresh("m1").unwrap();
    wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
    assert_eq!(fake.count("AT^MONNC"), 1);
    assert_eq!(panel.status(None).unwrap()["radio"]["partial"], true);
    assert_eq!(fake.count("AT^MONSC"), 2);
    panel.stop(Duration::from_secs(1)).unwrap();
}
#[test]
fn discovery_failure_does_not_erase_cached_inventory_or_measurements() {
    let fake = Fake::new();
    let panel = Sampler::new(fake.clone());
    panel.tick().unwrap();
    wait(|| panel.status(None).unwrap()["radio"]["pending"] == false);
    fake.discovery_error.store(true, Ordering::Release);
    assert!(panel.discover().is_err());
    let data = panel.status(None).unwrap();
    assert_eq!(data["selectedModem"], "m1");
    assert_eq!(data["radio"]["serving"][0]["rsrp"], -81.0);
    assert_eq!(data["radio"]["carriers"].as_array().unwrap().len(), 2);
    panel.stop(Duration::from_secs(1)).unwrap();
}
#[cfg(unix)]
mod linux {
    use super::*;
    use std::{io::Write, os::unix::net::UnixStream};
    use wlmb_server::transport::{self, Broker, Bus, Discovery};
    #[test]
    #[ignore = "Requires the isolated OpenWrt lab marker and real ubus, never a supply router"]
    fn native_openwrt_readonly_bridge_and_process() {
        assert!(std::path::Path::new("/etc/network-panel-isolated-lab").is_file());
        let bus = transport::NativeBus::default();
        let board = bus
            .call("system", "board", json!({}), Duration::from_secs(5))
            .unwrap();
        assert!(board["board_name"].is_string());
        let before = std::fs::read_dir("/proc/self/fd").unwrap().count();
        for _ in 0..25 {
            assert!(
                bus.call("system", "board", json!({}), Duration::from_secs(5))
                    .unwrap()
                    .is_object()
            );
        }
        assert_eq!(std::fs::read_dir("/proc/self/fd").unwrap().count(), before);
        assert!(
            bus.call(
                "network.interface.lan",
                "status",
                json!({}),
                Duration::from_secs(5)
            )
            .unwrap()["ipv4-address"]
                .is_array()
        );
        assert!(
            bus.call("evil", "call", json!({}), Duration::from_secs(5))
                .is_err()
        );
        assert!(
            transport::process(
                "/sbin/uci",
                &["-q", "show", "network.lan.proto"],
                None,
                Duration::from_secs(3)
            )
            .unwrap()
            .contains("network.lan.proto=")
        );
        assert!(
            transport::process(
                "/sbin/uci",
                &["-q", "batch"],
                Some(b"get network.lan.proto\n"),
                Duration::from_secs(3)
            )
            .unwrap()
            .contains("static")
        );
        assert_eq!(std::fs::read_dir("/proc/self/fd").unwrap().count(), before);
        bus.close().unwrap();
        assert!(
            bus.call("system", "board", json!({}), Duration::from_secs(5))
                .is_err()
        );
    }
    #[test]
    fn framed_ipc_bounds_fragmentation_eof_and_deadline() {
        let (mut read, mut write) = UnixStream::pair().unwrap();
        let writer = thread::spawn(move || {
            for raw in [b"000".as_slice(), b"00007", b"{\"a\":1}"] {
                write.write_all(raw).unwrap();
                thread::sleep(Duration::from_millis(2));
            }
        });
        assert_eq!(
            transport::read_frame(&mut read, Instant::now() + Duration::from_secs(1)).unwrap(),
            json!({"a":1})
        );
        writer.join().unwrap();
        for header in ["00040001", "00000000", "FFFFFFFF", "000000xz"] {
            let (mut read, mut write) = UnixStream::pair().unwrap();
            write.write_all(header.as_bytes()).unwrap();
            assert!(
                transport::read_frame(&mut read, Instant::now() + Duration::from_secs(1)).is_err()
            );
        }
        let (mut read, _write) = UnixStream::pair().unwrap();
        let start = Instant::now();
        assert!(transport::read_frame(&mut read, start + Duration::from_millis(25)).is_err());
        assert!(start.elapsed() < Duration::from_millis(200));
    }
    #[test]
    fn config_is_literal_strict_and_honors_shared_override() {
        let raw = "qmodem.main=main\nqmodem.main.at_tool='0'\nqmodem.m1=modem\nqmodem.m1.name='mt5700m-cn'\nqmodem.m1.at_port='/dev/ttyUSB1'\nqmodem.m1.override_at_port='/dev/ttyUSB9'\nqmodem.m1.use_ubus='1'\n";
        let rows = transport::parse_config(raw);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["port"], "/dev/ttyUSB9");
        assert_eq!(rows[0]["shared"], true);
        assert_eq!(
            transport::parse_config(&raw.replace("at_tool='0'", "at_tool='1'"))[0]["shared"],
            false
        );
        for bad in [
            "/dev/ttyUSB1;touch",
            "/dev/../ttyUSB1",
            "/dev/wwan0at",
            "/dev/mhi_",
            "ttyUSB1",
        ] {
            assert!(!transport::valid_port(bad));
        }
        assert!(transport::valid_port("/dev/wwan0at0"));
        assert!(transport::parse_config(&raw.replace("/dev/ttyUSB9", "$(malicious)")).is_empty());
    }
    struct Daemon {
        calls: Mutex<Vec<String>>,
        result: Result<Value>,
        gate: Option<Arc<Gate>>,
    }
    impl Bus for Daemon {
        fn call(
            &self,
            _object: &str,
            method: &str,
            _args: Value,
            _timeout: Duration,
        ) -> Result<Value> {
            self.calls.lock().unwrap().push(method.into());
            if method == "list" {
                Ok(json!({"ports":[{"port":"/dev/ttyUSB1","is_open":1}]}))
            } else {
                if let Some(gate) = &self.gate {
                    gate.hold();
                }
                self.result.clone()
            }
        }
    }
    struct Config {
        rows: Vec<Value>,
        owners: Result<Vec<String>>,
    }
    impl Discovery for Config {
        fn modems(&self) -> Result<Vec<Value>> {
            Ok(self.rows.clone())
        }
        fn owners(&self, _port: &str) -> Result<Vec<String>> {
            self.owners.clone()
        }
    }
    fn daemon(result: Result<Value>) -> Arc<Daemon> {
        Arc::new(Daemon {
            calls: Mutex::new(vec![]),
            result,
            gate: None,
        })
    }
    #[test]
    fn broker_fails_closed_on_unknown_or_conflicting_owners() {
        for owners in [
            Ok(vec![]),
            Ok(vec!["other-tool".into()]),
            Err(PanelError::new("AT_OWNERSHIP_UNKNOWN", 409)),
        ] {
            let bus = daemon(Ok(json!({"status":"success","response":"OK"})));
            let broker = Broker::new(
                bus.clone(),
                Arc::new(Config {
                    rows: vec![modem("m1")],
                    owners,
                }),
            );
            assert!(broker.query("m1", "AT^MONSC", None).is_err());
            assert_eq!(*bus.calls.lock().unwrap(), vec!["list"]);
        }
    }
    #[test]
    fn broker_does_not_replay_send_failure_and_enforces_cooldown() {
        let bus = daemon(Err(PanelError::new("SYSTEM_UNAVAILABLE", 503)));
        let broker = Broker::new(
            bus.clone(),
            Arc::new(Config {
                rows: vec![modem("m1")],
                owners: Ok(vec!["ubus-at-daemon".into()]),
            }),
        );
        assert!(
            broker
                .query("m1", "AT^MONSC", Some(("/dev/ttyUSB1", "mt5700m-cn")))
                .is_err()
        );
        assert_eq!(
            broker.query("m1", "AT^MONSC", None).unwrap_err().code,
            "AT_COOLDOWN"
        );
        assert_eq!(*bus.calls.lock().unwrap(), vec!["list", "sendat"]);
    }
    #[test]
    fn broker_checks_current_identity_shared_state_and_duplicate_port() {
        let bus = daemon(Ok(json!({"status":"success","response":"OK"})));
        let mut conflicting = modem("m2");
        conflicting["shared"] = json!(false);
        let broker = Broker::new(
            bus.clone(),
            Arc::new(Config {
                rows: vec![modem("m1"), conflicting],
                owners: Ok(vec!["ubus-at-daemon".into()]),
            }),
        );
        assert_eq!(
            broker
                .query("m1", "AT^MONSC", Some(("/dev/ttyUSB9", "mt5700m-cn")))
                .unwrap_err()
                .code,
            "MODEM_CHANGED"
        );
        assert_eq!(
            broker.query("m1", "AT^MONSC", None).unwrap_err().code,
            "AT_PORT_CONFLICT"
        );
        assert!(bus.calls.lock().unwrap().is_empty());
    }
    #[test]
    fn broker_serializes_same_port_without_replaying_contenders() {
        let gate = Arc::new(Gate::default());
        let bus = Arc::new(Daemon {
            calls: Mutex::new(vec![]),
            result: Ok(json!({"status":"success","response":"OK"})),
            gate: Some(gate.clone()),
        });
        let broker = Arc::new(Broker::new(
            bus.clone(),
            Arc::new(Config {
                rows: vec![modem("m1")],
                owners: Ok(vec!["ubus-at-daemon".into()]),
            }),
        ));
        let other = broker.clone();
        let job = thread::spawn(move || other.query("m1", "AT^MONSC", None).unwrap());
        wait(|| bus.calls.lock().unwrap().len() == 2);
        assert_eq!(
            broker.query("m1", "AT^MONSC", None).unwrap_err().code,
            "AT_BUSY"
        );
        gate.release();
        assert_eq!(job.join().unwrap(), "OK");
        assert_eq!(*bus.calls.lock().unwrap(), vec!["list", "sendat"]);
    }
}
