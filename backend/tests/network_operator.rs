#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::json;
use std::{collections::BTreeMap, time::Instant};
use wlmb_server::{MAX_SAMPLE_QUERIES, catalog, controls, network_operator, radio, security};
#[test]
fn every_radio_parser_uses_registered_operator_without_brand_commands() {
    for parser in [
        "tdtech",
        "quectel",
        "quectel-unisoc",
        "simcom",
        "fibocom-fm350",
        "fibocom-qualcomm",
        "fibocom-nl668",
        "openluat-air724",
        "sierra-em91",
        "sierra-em92",
        "sierra-lte",
        "generic",
    ] {
        let data = radio::parse(parser, "+COPS: 0,0,\"CHN-UNICOM\",12\nOK\n").unwrap();
        assert_eq!(data["operator"], "中国联通", "{parser}");
        assert_eq!(data["networkOperator"]["source"], "AT+COPS?");
        assert!(data["networkOperator"]["mcc"].is_null());
    }
}
#[test]
fn numeric_cops_keeps_three_digit_mnc_and_never_reads_sim_home_identity() {
    let data = radio::parse("generic", "+COPS: 0,2,\"310260\",7\nOK\n").unwrap();
    assert_eq!(data["networkOperator"]["mcc"], "310");
    assert_eq!(data["networkOperator"]["mnc"], "260");
    assert_eq!(data["operator"], "310260");
}
#[test]
fn nsa_anchor_plmn_supplies_operator_when_nr_row_has_no_plmn() {
    let mut data = json!({"serving":[{"rat":"LTE","mcc":"460","mnc":"01","source":"AT+QENG=servingcell"},{"rat":"NR"}]});
    network_operator::attach("", &mut data);
    assert_eq!(data["operator"], "中国联通");
    assert_eq!(data["networkOperator"]["source"], "AT+QENG=servingcell");
}
#[test]
fn conflicting_or_malformed_operator_does_not_replace_serving_network() {
    let mut data = json!({"serving":[{"mcc":"460","mnc":"01","source":"AT^MONSC"}]});
    network_operator::attach("+COPS: 0,2,\"46000\",7\nOK\n", &mut data);
    assert_eq!(data["operator"], "中国联通");
    assert_eq!(data["networkOperator"]["conflict"], true);
    for raw in [
        "+COPS: 2,0,\"CHN-UNICOM\"\nOK\n",
        "+COPS: 0,2,\"4601\"\nOK\n",
        "+COPS: 0,2,\"46001\"\n+COPS: 0,2,\"46000\"\nOK\n",
        "+COPS: 0\nOK\n",
    ] {
        assert!(radio::parse("generic", raw).unwrap()["operator"].is_null());
    }
}
#[test]
fn all_upstream_profiles_have_distinct_band_facts_without_lock_authority() {
    let bands: serde_json::Value =
        serde_json::from_str(include_str!("../../docs/model-bands.json")).unwrap();
    let upstream: serde_json::Value =
        serde_json::from_str(include_str!("../../docs/qmodem-upstream.json")).unwrap();
    assert_eq!(bands["profiles"].as_array().unwrap().len(), 131);
    assert_eq!(bands["revision"], upstream["revision"]);
    for row in upstream["models"].as_array().unwrap() {
        assert!(
            bands["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["model"] == row["model"] && r["transport"] == row["transport"])
        );
        let preset = controls::presets(row["model"].as_str().unwrap());
        assert_eq!(
            preset["bandLockProtocolSupported"],
            wlmb_server::lock_protocol::band_supported(row["model"].as_str().unwrap())
        );
        assert_eq!(preset["notHardwareVerified"], true);
        if !wlmb_server::lock_protocol::cell_supported(row["model"].as_str().unwrap()) {
            assert_eq!(preset["lockProtocolSupported"], false);
            assert!(
                controls::validate_preset(
                    row["model"].as_str().unwrap(),
                    &json!({"rat":"NR","mode":"auto","entries":[]})
                )
                .is_err()
            );
        }
    }
    assert!(
        controls::presets("rm520n-gl")["NR"]
            .as_array()
            .unwrap()
            .contains(&json!(77))
    );
    assert!(
        controls::presets("em12")["NR"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn operator_queries_are_read_only_slow_and_bounded_for_all_implemented_profiles() {
    for (_, models) in catalog::MODELS {
        for model in models.split_whitespace() {
            let p = catalog::identify(model);
            let queries = p["queries"].as_array().unwrap();
            assert!(queries.len() <= MAX_SAMPLE_QUERIES);
            assert_eq!(
                queries.iter().any(|q| q == "AT+COPS?"),
                p["parser"].is_string()
            );
            assert!(
                queries
                    .iter()
                    .all(|q| security::read_only(q.as_str().unwrap()))
            );
            assert!(
                !queries
                    .iter()
                    .any(|q| q.as_str().unwrap().starts_with("AT+COPS="))
            );
        }
    }
    assert_eq!(catalog::query_ttl("AT+COPS?", 3, false), 60);
}
#[test]
fn operator_freshness_tracks_its_own_source_and_clears_legacy_name() {
    use wlmb_server::sampling::{self, Sample};
    let primary = "AT^MONSC".to_owned();
    let commands = vec![primary.clone(), "AT+COPS?".to_owned()];
    let modem = json!({"profile":{"model":"mt5700m-cn","parser":"tdtech"}});
    let mut samples = BTreeMap::new();
    samples.insert(
        primary,
        Sample {
            raw: "^MONSC: NR,460,01,627264,1,abc,1E8,abc,-80,-5,16\nOK\n".into(),
            wall_ms: 84000,
            mono: Instant::now(),
        },
    );
    let data = sampling::publish(&modem, &commands, &samples, &[], 100000).unwrap();
    assert_eq!(data["networkOperator"]["stale"], true);
    assert!(data["operator"].is_null());
    let modem = json!({"profile":{"model":"unknown","parser":"generic"}});
    let commands = vec!["AT+CSQ".into(), "AT+COPS?".into()];
    samples.clear();
    samples.insert(
        "AT+CSQ".into(),
        Sample {
            raw: "+CSQ: 20,99\nOK\n".into(),
            wall_ms: 100000,
            mono: Instant::now(),
        },
    );
    samples.insert(
        "AT+COPS?".into(),
        Sample {
            raw: "+COPS: 0,0,\"CHN-UNICOM\",12\nOK\n".into(),
            wall_ms: 40000,
            mono: Instant::now(),
        },
    );
    let data = sampling::publish(&modem, &commands, &samples, &[], 100000).unwrap();
    assert_eq!(data["networkOperator"]["stale"], false);
    assert_eq!(data["networkOperator"]["observedAtMs"], 40000);
    assert_eq!(data["operator"], "中国联通");
    assert!(
        sampling::publish(&modem, &commands, &samples, &[], 140000).unwrap()["operator"].is_null()
    );
}
#[test]
fn quectel_serving_operator_timestamp_normalizes_quoted_query_identity() {
    use wlmb_server::sampling::{self, Sample};
    let key = "AT+QENG=\"servingcell\"".to_owned();
    let mut samples = BTreeMap::new();
    samples.insert(key.clone(),Sample{raw:"+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,01,5F1EA15,12,1650,3,5,5,DE10,-100,-12,-68,11\nOK\n".into(),wall_ms:100000,mono:Instant::now()});
    let modem = json!({"profile":{"model":"rm520n-gl","parser":"quectel"}});
    let data = sampling::publish(&modem, &[key], &samples, &[], 100500).unwrap();
    assert_eq!(data["operator"], "中国联通");
    assert_eq!(data["networkOperator"]["stale"], false);
    assert_eq!(data["networkOperator"]["observedAtMs"], 100000);
}
