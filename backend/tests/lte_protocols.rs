#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::{catalog, radio, security};
const NL: &str = "+GTCCINFO:\r\nLTE service cell:\r\n1,4,460,01,6334,140542123,39148,351,40,5,14,51,51,29\r\n2,4,460,01,6334,140541985,38950,352,5,57,57,24\r\nOK\r\n";
const AIR_MAIN: &str = "AT+CCED=0,1\r\n+CCED:LTE current cell:460,00,460045926307603,0,40,n100,39148,140542123,51,29,6334,351,351\r\nOK\r\n";
const AIR_NB: &str = "AT+CCED=0,2\r\n+CCED:LTE neighbor cell:460,00,38950,140541985,57,24,6334,36,352\r\n+CCED:LTE neighbor cell:460,00,1300,26224401,48,24,6334,27,37\r\nOK\r\n";
#[test]
fn nl668_uses_decimal_addresses_half_db_sinr_and_not_fm_bandwidth_or_rssi() {
    let v = radio::parse("fibocom-nl668", NL).unwrap();
    let s = &v["serving"][0];
    assert_eq!(s["channel"], 39148.);
    assert_eq!(s["pci"], 351.);
    assert_eq!(s["band"], "B40");
    assert_eq!(s["rsrp"], -90.);
    assert_eq!(s["rsrq"], -5.5);
    assert_eq!(s["sinr"], 7.);
    assert!(s["rssi"].is_null());
    assert!(s["dlBandwidthMHz"].is_null());
    assert!(s["ulBandwidthMHz"].is_null());
    assert_eq!(v["carriers"].as_array().unwrap().len(), 1);
    assert_eq!(v["neighbors"][0]["rsrp"], -84.);
    assert_eq!(v["neighbors"][0]["bandCandidates"][0], "B40");
    assert!(v["neighbors"][0]["sinr"].is_null());
    assert_eq!(s["metricIntervals"]["sinr"]["upperInclusive"], 7.);
}
#[test]
fn nl668_saturated_unknown_and_cross_platform_fields_remain_unknown() {
    let v = radio::parse(
        "fibocom-nl668",
        &NL.replace(",14,51,51,29", ",255,99,0,255"),
    )
    .unwrap();
    for key in ["rssi", "rsrp", "rsrq", "sinr"] {
        assert!(v["serving"][0][key].is_null());
    }
    for bad in [
        NL.replace(",39148,351,40,", ",98ECC,15F,40,"),
        NL.replace(",351,40,", ",351,3,"),
        NL.replace(",4,460", ",9,460"),
    ] {
        assert!(
            radio::parse("fibocom-nl668", &bad).unwrap()["serving"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(catalog::identify("nl678")["parser"], "fibocom-hex");
}
#[test]
fn nl668_incomplete_failed_and_ambiguous_primary_frames_never_create_ca() {
    for raw in [
        NL.replace("OK", ""),
        NL.replace("OK", "ERROR"),
        NL.replace("OK", "+CME ERROR: 4\nOK"),
        NL.replace(
            "LTE service cell:",
            "1,4,460,01,6334,140542123,39148,351,40,5,14,51,51,29",
        ),
    ] {
        assert!(
            radio::parse("fibocom-nl668", &raw).unwrap()["carriers"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
#[test]
fn air724_one_shot_records_export_no_imsi_and_do_not_invent_sinr_ul_or_ca() {
    let raw = format!("{AIR_MAIN}{AIR_NB}");
    let v = radio::parse("openluat-air724", &raw).unwrap();
    let s = &v["serving"][0];
    assert_eq!(s["band"], "B40");
    assert_eq!(s["rsrp"], -90.);
    assert_eq!(s["rsrq"], -5.5);
    assert_eq!(s["dlBandwidthMHz"], 20.);
    assert!(s["ulBandwidthMHz"].is_null());
    assert!(s["rssi"].is_null());
    assert!(s["sinr"].is_null());
    assert_eq!(v["carriers"].as_array().unwrap().len(), 1);
    assert_eq!(v["neighbors"].as_array().unwrap().len(), 2);
    assert_eq!(v["neighbors"][0]["pci"], 352.);
    assert_eq!(v["neighbors"][0]["rsrp"], -84.);
    assert!(!v.to_string().contains("460045926307603"));
    let p = catalog::identify("air724ug");
    assert_eq!(p["parser"], "openluat-air724");
    assert_eq!(
        p["queries"],
        serde_json::json!(["AT+CCED=0,1", "AT+CCED=0,2", "AT+COPS?"])
    );
    for cmd in ["AT+CCED=0,1", "AT+CCED=0,2", "AT+COPS?"] {
        assert!(security::read_only(cmd));
    }
    for cmd in ["AT+CCED=1,1", "AT+CCED=1,2", "AT+CCED=2,1"] {
        assert!(!security::read_only(cmd));
    }
}
#[test]
fn air724_failed_optional_query_does_not_erase_good_serving_or_fake_unknown_width() {
    let v = radio::parse(
        "openluat-air724",
        &format!(
            "{}{}",
            AIR_MAIN.replace("n100", "100"),
            AIR_NB.replace("OK", "ERROR")
        ),
    )
    .unwrap();
    assert_eq!(v["serving"].as_array().unwrap().len(), 1);
    assert!(v["serving"][0]["dlBandwidthMHz"].is_null());
    assert!(v["neighbors"].as_array().unwrap().is_empty());
    for raw in [
        AIR_MAIN.replace("OK", ""),
        AIR_MAIN.replace("39148", "999999"),
        AIR_MAIN.replace(",40,n100", ",3,n100"),
        format!("{AIR_MAIN}{AIR_MAIN}"),
    ] {
        assert!(
            radio::parse("openluat-air724", &raw).unwrap()["serving"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
