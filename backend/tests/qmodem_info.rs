#![allow(clippy::unwrap_used)]
// 参考 QModem 公开字段与用户反馈的结构，测试仅使用合成小区数据。
use serde_json::{Value, json};
use wlmb_server::qmodem_info;

fn rows(pairs: &[(&str, &str)]) -> Value {
    json!({"modem_info":pairs.iter().map(|(key,value)|json!({"key":key,"value":value})).collect::<Vec<_>>()})
}
fn cell(mode: &str, channel: &str) -> Value {
    rows(&[
        ("network_mode", mode),
        ("MCC", "460"),
        ("MNC", "00"),
        ("TAC", "ABCDE"),
        ("Cell ID", "1234ABCD"),
        ("Physical Cell ID", "214"),
        (channel, "152650"),
        ("Band", "28"),
        ("DL Bandwidth", "30"),
        ("UL Bandwidth", "30"),
        ("RSRP", "-94"),
        ("RSRQ", "-16"),
        ("SINR", "7"),
        ("SCS", "15"),
    ])
}

#[test]
fn sa_feedback_shape_maps_without_model_specific_routing_or_private_data() {
    let network = rows(&[
        ("AMBR DL", "3000"),
        ("AMBR UL", "200"),
        ("CQI DL", "6"),
        ("CQI UL", "6"),
        ("Rx Rate", "973.80 Kbps"),
        ("IMEI", "synthetic-private-field"),
    ]);
    let value = qmodem_info::parse(&cell("NR5G-SA Mode", "ARFCN"), Some(&network), 1000).unwrap();
    let serving = &value["serving"][0];
    assert_eq!(serving["rsrp"], -94.);
    assert_eq!(serving["rsrq"], -16.);
    assert_eq!(serving["sinr"], 7.);
    assert_eq!(serving["band"], "n28");
    assert_eq!(serving["scsKHz"], 15.);
    assert_eq!(serving["dlBandwidthMHz"], 30.);
    assert_eq!(serving["observedAtMs"], 1000);
    assert_eq!(value["operator"], "中国移动");
    assert_eq!(value["subscription"]["dlKbps"], 3000000.);
    assert_eq!(value["subscription"]["ulKbps"], 200000.);
    assert_eq!(value["subscription"]["cqiDl"], 6.);
    assert!(value["sessionQos"].is_null());
    assert_eq!(value["carriers"].as_array().unwrap().len(), 1);
    assert_eq!(value["carrierState"]["status"], "serving-only");
    assert_eq!(value["carrierState"]["inventoryComplete"], false);
    assert!(!value.to_string().contains("synthetic-private-field"));
}

#[test]
fn formatted_contract_rates_preserve_units_and_reject_unknown_encodings() {
    for (dl, ul, expected_dl, expected_ul) in [
        ("3 Gbps", "200 Mbps", 3000000., 200000.),
        ("3000000 Kbps", "25000 KB/s", 3000000., 200000.),
        ("3000000000 bps", "25 MB/s", 3000000., 200000.),
    ] {
        let mut value = json!({});
        qmodem_info::attach_network(&rows(&[("AMBR DL", dl), ("AMBR UL", ul)]), &mut value, 1)
            .unwrap();
        assert_eq!(value["subscription"]["dlKbps"], expected_dl);
        assert_eq!(value["subscription"]["ulKbps"], expected_ul);
    }
    let mut value = json!({});
    qmodem_info::attach_network(
        &rows(&[("AMBR DL", "3 strange"), ("AMBR UL", "200")]),
        &mut value,
        1,
    )
    .unwrap();
    assert!(value["subscription"].is_null());
}

#[test]
fn lte_nsa_and_ca_do_not_mix_fields_or_make_ambiguous_carriers() {
    let lte = qmodem_info::parse(&cell("LTE Mode", "EARFCN"), None, 1000).unwrap();
    assert_eq!(lte["serving"][0]["rat"], "LTE");
    assert_eq!(lte["serving"][0]["band"], "B28");
    let mut nsa = rows(&[
        ("network_mode", "EN-DC Mode"),
        ("LTE", "LTE"),
        ("MCC", "460"),
        ("MNC", "01"),
        ("EARFCN", "1650"),
        ("Physical Cell ID", "40"),
        ("Band", "3"),
        ("RSRP", "-95"),
        ("NR5G-NSA", "NR5G-NSA"),
        ("ARFCN", "627264"),
        ("Physical Cell ID", "214"),
        ("Band", "78"),
        ("RSRP", "-85"),
        ("RSRQ", "-10"),
        ("SINR", "15"),
        ("ARFCN (CA)", "630000 / 636664"),
        ("Band (CA)", "78 / 78"),
        ("Physical Cell ID (CA)", "20 / 21"),
        ("DL Bandwidth (CA)", "100 / 80"),
    ]);
    let value = qmodem_info::parse(&nsa, None, 2000).unwrap();
    assert_eq!(value["serving"].as_array().unwrap().len(), 2);
    assert_eq!(value["serving"][0]["rsrp"], -95.);
    assert_eq!(value["serving"][1]["rsrp"], -85.);
    assert_eq!(value["serving"][1]["role"], "nsa");
    assert_eq!(value["carriers"][2]["dlBandwidthMHz"], 100.);
    assert_eq!(value["carriers"][3]["dlBandwidthMHz"], 80.);
    nsa["modem_info"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["key"] == "Band (CA)")
        .unwrap()["value"] = json!("78");
    let incomplete = qmodem_info::parse(&nsa, None, 2000).unwrap();
    assert_eq!(incomplete["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(incomplete["carrierState"]["status"], "serving-only");
}

#[test]
fn invalid_empty_duplicate_and_out_of_range_values_are_not_fabricated() {
    for invalid in [
        json!({}),
        json!({"modem_info":[]}),
        rows(&[("network_mode", "not registered")]),
        rows(&[
            ("network_mode", "NR5G-SA Mode"),
            ("ARFCN", "627264"),
            ("ARFCN", "630000"),
        ]),
    ] {
        assert!(qmodem_info::parse(&invalid, None, 1).is_err());
    }
    let value = rows(&[
        ("network_mode", "NR5G-SA Mode"),
        ("ARFCN", "627264"),
        ("Physical Cell ID", "-1"),
        ("RSRP", "255"),
        ("SINR", "nan"),
        ("Band", "-"),
    ]);
    let value = qmodem_info::parse(&value, None, 1).unwrap();
    assert!(value["serving"][0]["pci"].is_null());
    assert!(value["serving"][0]["rsrp"].is_null());
    assert!(value["serving"][0]["sinr"].is_null());
    assert!(value["serving"][0]["band"].is_null());
    assert!(value["operator"].is_null());
    assert!(qmodem_info::supported(
        &json!({"cell_info":{"config_section":3}})
    ));
    assert!(!qmodem_info::supported(
        &json!({"cell_info":{"config_section":5}})
    ));
}

#[test]
fn optional_temperature_accepts_units_and_never_leaks_other_fields() {
    for raw in ["47°C", "47℃", "47 C", "47"] {
        let mut value = json!({});
        qmodem_info::attach_base(
            &rows(&[("temperature", raw), ("IMSI", "synthetic-private-field")]),
            &mut value,
            123,
        )
        .unwrap();
        assert_eq!(value["temperature"]["celsius"], 47.);
        assert_eq!(value["temperature"]["observedAtMs"], 123);
        assert!(!value.to_string().contains("synthetic-private-field"));
    }
    let mut value = json!({});
    qmodem_info::attach_base(&rows(&[("temperature", "255°C")]), &mut value, 123).unwrap();
    assert!(value["temperature"].is_null());
}
