#![allow(clippy::unwrap_used)]
// 接入说明的合成样本；不包含真实设备身份，不调用路由器或真实事务工具。
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use wlmb_server::{
    PanelError, Result,
    luoye::{self, Input, Io, Luoye, ToolReply},
    luoye_info,
    sampler::{Sampler, Source},
};
fn fixture() -> Value {
    json!({
        "base_info":{"name":"RM520F-GL","manufacturer":"quectel","at_port":"/dev/modem-at","temperature":"47°C","connect_status":"connect"},
        "sim_info":[{"SIM Status":"ready"},{"IMSI":"private-identity-not-for-output"},{"ICCID":"private-card-not-for-output"}],
        "network_info":[{"AMBR DL":"2000 Mbps"},{"AMBR UL":"200 Mbps"},{"CQI DL":"0"}],
        "cell_info":{"NR5G-SA Mode":[{"ARFCN":"633984"},{"Physical Cell ID":"0"},{"Band":"N78"},
            {"MCC":"460"},{"MNC":"01"},{"Cell ID":"ABC123456789ABCDE"},{"TAC":"0011"},
            {"RSRP":"-85 dBm"},{"RSRQ":"-10 dB"},{"SINR":"0 dB"},{"DL Bandwidth":"100 MHz"},
            {"SCS":"30 kHz"},{"DL MCS":"0"},{"CSI CQI":"0"},
            {"Carrier Aggregation":"PCC | SCC"},{"CA Band":"N78 | N41"},{"CA DL ARFCN":"633984 | 519000"},
            {"CA DL Bandwidth":"100 | 80"},{"CA UL ARFCN":"- | 519000"},{"CA UL Bandwidth":"- | 40"}]}
    })
}
fn parse(value: &Value) -> Value {
    luoye_info::parse(value, Some(1000), true, 2000).unwrap()
}
#[test]
fn cache_fields_preserve_units_zero_plmn_and_large_identity_without_leaking_sim_identity() {
    let result = parse(&fixture());
    assert_eq!(result["serving"][0]["pci"], 0.);
    assert_eq!(result["serving"][0]["sinr"], 0.);
    assert_eq!(result["serving"][0]["mnc"], "01");
    assert_eq!(result["serving"][0]["cellId"], "ABC123456789ABCDE");
    assert_eq!(result["serving"][0]["rsrp"], -85.);
    assert_eq!(result["serving"][0]["csi"]["dlMcs"], 0.);
    assert_eq!(result["serving"][0]["csi"]["cqi"], 0.);
    assert_eq!(result["networkOperator"]["name"], "中国联通");
    assert_eq!(result["subscription"]["dlKbps"], 2000000.);
    assert_eq!(result["subscription"]["cqiDl"], 0.);
    assert_eq!(result["temperature"]["celsius"], 47.);
    assert!(!result.to_string().contains("private-"));
    assert_eq!(result["observedAtMs"], 1000);
    assert_eq!(result["receivedAtMs"], 2000);
    assert_eq!(result["observationTimeBasis"], "source-file-publication");
}
#[test]
fn ca_columns_preserve_roles_and_missing_ul_without_copying_serving_signals() {
    let result = parse(&fixture());
    let rows = result["carriers"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["role"], "pcc");
    assert_eq!(rows[1]["role"], "scc");
    assert_eq!(rows[1]["band"], "n41");
    assert!(rows[0]["ulChannel"].is_null());
    assert!(rows[0]["ulBandwidthMHz"].is_null());
    assert_eq!(rows[1]["ulChannel"], 519000.);
    for row in rows {
        assert!(row["rsrp"].is_null());
        assert!(row["sinr"].is_null());
        assert!(row["pci"].is_null());
    }
    let mut invalid = fixture();
    invalid["cell_info"]["NR5G-SA Mode"][15] = json!({"CA Band":"N78"});
    let result = parse(&invalid);
    assert_eq!(result["partial"], true);
    assert_eq!(result["carriers"].as_array().unwrap().len(), 1);
    assert_eq!(result["errors"][0]["code"], "MODEM_CACHE_CA_INVALID");
}
#[test]
fn en_dc_groups_are_keyed_not_positional_and_do_not_merge_lte_and_nr_measurements() {
    let mut value = fixture();
    value["cell_info"] = json!({"EN-DC Mode":[{"NR5G-NSA":[{"ARFCN":"633984"},{"Physical Cell ID":"0"},{"RSRP":"-85"}]},
        {"LTE":[{"EARFCN":"1650"},{"Physical Cell ID":"22"},{"RSRP":"-97"}]}]});
    let result = parse(&value);
    let rows = result["serving"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["rat"], "LTE");
    assert_eq!(rows[0]["role"], "pcc");
    assert_eq!(rows[0]["rsrp"], -97.);
    assert_eq!(rows[1]["rat"], "NR");
    assert_eq!(rows[1]["role"], "nsa");
    assert_eq!(rows[1]["rsrp"], -85.);
}
#[test]
fn cache_invalid_duplicate_fields_html_and_unknown_units_do_not_become_numbers() {
    assert!(luoye_info::payload("{}").is_err());
    assert!(luoye_info::payload("<html>login</html>").is_err());
    let value = fixture();
    assert_eq!(
        luoye_info::payload(&json!({"modem_info":{"more_info":value}}).to_string()).unwrap(),
        value
    );
    let mut invalid = fixture();
    invalid["cell_info"]["NR5G-SA Mode"]
        .as_array_mut()
        .unwrap()
        .push(json!({"RSRP":"-90"}));
    assert!(luoye_info::parse(&invalid, Some(1000), true, 2000).is_err());
    let mut unknown = fixture();
    unknown["network_info"][0] = json!({"AMBR DL":"2000"});
    unknown["cell_info"]["NR5G-SA Mode"][7] = json!({"RSRP":"<b>-85</b> / -90"});
    let result = parse(&unknown);
    assert!(result["subscription"]["dlKbps"].is_null());
    assert!(result["serving"][0]["rsrp"].is_null());
}
#[test]
fn absent_future_old_or_unbound_publication_is_stale_and_does_not_claim_connected() {
    for (time, trusted, now) in [
        (None, true, 2000),
        (Some(1000), false, 2000),
        (Some(1000), true, 16001),
        (Some(3000), true, 2000),
    ] {
        let result = luoye_info::parse(&fixture(), time, trusted, now).unwrap();
        assert_eq!(result["stale"], true);
        assert_eq!(result["serving"][0]["stale"], true);
        assert!(result["connected"].is_null());
    }
    let mut missing = fixture();
    missing["cell_info"] = json!({});
    missing["sim_info"] = json!([{"SIM Status":"PIN required"}]);
    let result = parse(&missing);
    assert!(result["serving"].as_array().unwrap().is_empty());
    assert_eq!(result["simStatus"], "PIN required");
}
struct Fake {
    value: Mutex<Value>,
    at: u64,
    valid: bool,
    calls: AtomicUsize,
    fail: bool,
    change: bool,
    enabled: bool,
}
impl Fake {
    fn new() -> Self {
        Self {
            value: Mutex::new(fixture()),
            at: wlmb_server::events::wall_ms(),
            valid: true,
            calls: AtomicUsize::new(0),
            fail: false,
            change: false,
            enabled: true,
        }
    }
}
impl Io for Fake {
    fn neighbors_enabled(&self) -> bool {
        self.enabled
    }
    fn load(&self) -> Result<Input> {
        Ok(Input {
            raw: self.value.lock().unwrap().to_string(),
            published_at_ms: Some(self.at),
            instance: "hostboot:endpoint1".into(),
            identity_valid: self.valid,
        })
    }
    fn neighbor(&self, request: &Value) -> Result<ToolReply> {
        assert!(luoye::request_valid(request));
        assert_eq!(request["profile"], "quectel");
        assert_eq!(request["commands"], json!(["AT+QENG=\"neighbourcell\""]));
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(PanelError::new("MODEM_NEIGHBOR_RESULT_UNKNOWN", 503));
        }
        if self.change {
            self.value.lock().unwrap()["cell_info"]["NR5G-SA Mode"][1] =
                json!({"Physical Cell ID":"1"});
        }
        Ok(ToolReply {
            success: true,
            value: json!({"result":"\r\n+QENG: \"neighbourcell\",\"NR5G\",633984,30,-95,-13,5\r\nOK"}),
        })
    }
}
#[test]
fn original_transaction_is_neighbor_only_publishes_cache_first_and_shares_query_result() {
    let io = Arc::new(Fake::new());
    let source = Luoye::new(io.clone());
    let row = source.modems().unwrap().remove(0);
    assert_eq!(row["infoTransport"], luoye_info::PROVIDER);
    assert_eq!(row["infoNeighborQuery"], true);
    assert!(!wlmb_server::control_runtime::ModemControls::available(
        &row
    ));
    for command in [
        "AT+QENG=\"neighbourcell\"",
        "AT+CFUN=0",
        "AT+CMGL=4",
        "AT+QNWLOCK=\"common/5g\"",
    ] {
        assert!(source.query_bound(&row, command).is_err());
    }
    let mut progress = 0;
    let result = source
        .info_snapshot_progress(&row, &mut |value| {
            assert_eq!(io.calls.load(Ordering::SeqCst), 0);
            assert_eq!(value["serving"][0]["rsrp"], -85.);
            assert_eq!(value["neighborState"]["queryStatus"], "pending");
            progress += 1;
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(progress, 1);
    assert_eq!(result["neighbors"][0]["pci"], 30.);
    let at = result["neighbors"][0]["observedAtMs"].clone();
    let again = source.info_snapshot(&row).unwrap().unwrap();
    assert_eq!(again["neighbors"][0]["observedAtMs"], at);
    assert_eq!(io.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn unknown_result_is_not_requeued_and_serving_changes_discard_neighbors() {
    let mut fake = Fake::new();
    fake.fail = true;
    let io = Arc::new(fake);
    let source = Luoye::new(io.clone());
    let row = source.modems().unwrap().remove(0);
    for _ in 0..3 {
        let result = source.info_snapshot(&row).unwrap().unwrap();
        assert_eq!(result["neighborState"]["automaticRetryPaused"], true);
        assert!(result["neighbors"].as_array().unwrap().is_empty());
    }
    assert_eq!(io.calls.load(Ordering::SeqCst), 1);
    for manufacturer in ["unknown", "quectel"] {
        io.value.lock().unwrap()["base_info"]["manufacturer"] = json!(manufacturer);
        let current = source.modems().unwrap().remove(0);
        source.info_snapshot(&current).unwrap();
    }
    assert_eq!(io.calls.load(Ordering::SeqCst), 1);
    let mut fake = Fake::new();
    fake.change = true;
    let io = Arc::new(fake);
    let source = Luoye::new(io.clone());
    let row = source.modems().unwrap().remove(0);
    assert_eq!(
        source.info_snapshot(&row).unwrap_err().code,
        "MODEM_CHANGED"
    );
}
#[test]
fn stale_unknown_vendor_unbound_or_disabled_cache_never_starts_neighbor_transaction() {
    for case in 0..5 {
        let mut fake = Fake::new();
        match case {
            0 => fake.at = 1,
            1 => fake.valid = false,
            2 => fake.enabled = false,
            3 => fake.value.get_mut().unwrap()["base_info"]["manufacturer"] = json!("unknown"),
            _ => fake.value.get_mut().unwrap()["base_info"]["name"] = json!("unknown"),
        };
        let io = Arc::new(fake);
        let source = Luoye::new(io.clone());
        let row = source.modems().unwrap().remove(0);
        source.info_snapshot(&row).unwrap();
        assert_eq!(io.calls.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn request_and_response_guards_reject_free_at_partial_frames_and_successful_http_only() {
    let request = json!({"port":"/dev/modem-at","profile":"quectel","commands":["AT+QENG=\"neighbourcell\""],"timeout":4,"baud":115200});
    assert!(luoye::request_valid(&request));
    for (key, value) in [
        ("commands", json!(["AT+CFUN=0"])),
        (
            "commands",
            json!(["AT+QENG=\"neighbourcell\"", "AT+CFUN=0"]),
        ),
        ("port", json!("/dev/ttyUSB2")),
        ("profile", json!("")),
        ("timeout", json!(60)),
        ("extra", json!(true)),
    ] {
        let mut invalid = request.clone();
        invalid[key] = value;
        assert!(!luoye::request_valid(&invalid));
    }
    for value in [
        json!({"success":true,"data":"OK"}),
        json!({"result":"+QENG: x"}),
        json!({"result":"OK\nOK"}),
        json!({"result":"OK","error":"AT response timed out"}),
        json!({"result":"+QENG: \"servingcell\",x\nOK"}),
    ] {
        assert!(
            luoye::response(
                &ToolReply {
                    success: true,
                    value
                },
                "AT+QENG=\"neighbourcell\""
            )
            .is_err()
        );
    }
}
#[test]
fn sampler_consumes_cache_provider_without_entering_general_at_collection() {
    let mut fake = Fake::new();
    fake.enabled = false;
    let source = Luoye::new(Arc::new(fake));
    let sampler = Sampler::new(source);
    sampler.discover().unwrap();
    sampler.refresh("luoye-modem").unwrap();
    for _ in 0..100 {
        let status = sampler.status(None).unwrap();
        if status["radio"]["serving"][0]["rsrp"] == -85. {
            assert!(status["radio"]["observedAtMs"].is_u64());
            assert!(sampler.stop(std::time::Duration::from_secs(1)).unwrap());
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("缓存数据源未进入采样器");
}
#[test]
fn installed_modem_manager_is_not_bypassed_even_when_cache_or_qmodem_are_unavailable() {
    use wlmb_server::connection_mode::{Backend, with_modem_cache};
    for requested in [None, Some(false), Some(true)] {
        for qmodem in [false, true] {
            assert_eq!(
                with_modem_cache(requested, qmodem, false, true),
                Backend::Luoye
            );
        }
    }
}
