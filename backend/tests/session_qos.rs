#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::{MAX_SAMPLE_QUERIES, catalog, radio, security, sms_settings};
#[test]
fn mt_sa_real_five_field_reply_and_matching_qci() {
    let data = radio::parse(
        "tdtech",
        "^DSAMBR: 1,2000000,200000,\"3gnet\",5\n+CGEQOSRDP: 1,6\n^IMSSWITCH: 0,0,0\nOK\n",
    )
    .unwrap();
    assert_eq!(data["subscription"]["dlKbps"], 2000000.);
    assert_eq!(data["subscription"]["ulKbps"], 200000.);
    assert_eq!(data["subscription"]["apn"], "3gnet");
    assert_eq!(data["sessionQos"]["qci"], 6);
    assert_eq!(data["smsReceiving"]["enabled"], false);
}
#[test]
fn cid_eight_is_not_dropped_and_ims_and_conflicting_qci_are_not_selected() {
    let data=radio::parse("tdtech","^DSAMBR: 1,1000,100,\"ims\",5\n^DSAMBR: 8,2000000,200000,\"internet\",5\n+CGEQOSRDP: 1,5\n+CGEQOSRDP: 8,9\n").unwrap();
    assert_eq!(data["subscription"]["cid"], 8);
    assert_eq!(data["sessionQos"]["qci"], 9);
    let data=radio::parse("tdtech","^DSAMBR: 1,2000000,200000,\"internet\",5\n^DSAMBR: 8,3000000,300000,\"vpn\",5\n+CGEQOSRDP: 1,9\n").unwrap();
    assert!(data["subscription"].is_null());
    assert!(data["sessionQos"].is_null());
    assert_eq!(data["subscriptionAmbiguous"], true);
}
#[test]
fn quectel_units_and_cqi_are_separate_from_qci() {
    let raw = "+QNWCFG: \"nr5g_ambr\",\"internet\",13,2000,12,200\n+CGCONTRDP: 3,5,\"internet\"\n+CGEQOSRDP: 3,9\n";
    let data = radio::parse("quectel", raw).unwrap();
    assert_eq!(data["subscription"]["dlKbps"], 2000000.);
    assert_eq!(data["subscription"]["cqiDl"], 13.);
    assert_eq!(data["sessionQos"]["qci"], 9);
    let data = radio::parse(
        "quectel",
        "+QNWCFG: \"nr5g_ambr\",\"internet\",13,2000,12,200\n",
    )
    .unwrap();
    assert!(data["sessionQos"].is_null());
    assert!(radio::parse("quectel-unisoc", raw).unwrap()["subscription"].is_null());
}
#[test]
fn standard_qci_requires_unambiguous_data_context_and_rates_are_not_ambr() {
    let raw = "+CGCONTRDP: 1,5,\"ims\"\n+CGCONTRDP: 3,5,\"internet\"\n+CGEQOSRDP: 1,5\n+CGEQOSRDP: 3,9,999999,888888\n";
    for parser in [
        "quectel-unisoc",
        "simcom",
        "fibocom-fm350",
        "sierra-em91",
        "openluat-air724",
    ] {
        let data = radio::parse(parser, raw).unwrap();
        assert_eq!(data["sessionQos"]["qci"], 9);
        assert!(data["subscription"].is_null());
    }
    let data = radio::parse("simcom", &format!("{raw}+CGCONTRDP: 4,5,\"vpn\"\n")).unwrap();
    assert!(data["sessionQos"].is_null());
    for raw in [
        "+CGEQOSRDP: 1,9\n",
        "^DSAMBR: 1,NaN,200000,\"internet\"\n",
        "^DSAMBR: 1,0,0,\"internet\"\n",
    ] {
        assert!(radio::parse("tdtech", raw).unwrap()["subscription"].is_null());
    }
}
#[test]
fn standard_explicit_apn_ambr_is_portable_and_dual_stack_contexts_are_deduplicated() {
    let raw = "+CGCONTRDP: 3,5,\"internet\",\"IPv4\"\n+CGCONTRDP: 3,5,\"internet\",\"IPv6\"\n+CGEQOSRDP: 3,9,,,,,2000000,200000\n";
    for parser in ["simcom", "fibocom-fm350", "sierra-em91", "quectel-unisoc"] {
        let value = radio::parse(parser, raw).unwrap();
        assert_eq!(value["subscription"]["dlKbps"], 2000000.);
        assert_eq!(value["subscription"]["kind"], "APN-AMBR");
        assert_eq!(value["sessionQos"]["qci"], 9);
    }
}
#[test]
fn partial_fixed_point_neighbors_keep_valid_fields_but_sentinels_remain_unknown() {
    let data=radio::parse("tdtech","^MONNC: NR,627264,D8,-720,-96,32767\n^MONNC: NR,627264,1B0,-157,-44,-24\n^MONNC: NR,627264,16F,-784,--,16\n").unwrap();
    let rows = data["neighbors"].as_array().unwrap();
    let first = rows.iter().find(|r| r["pci"] == 216.).unwrap();
    let missing = rows.iter().find(|r| r["pci"] == 432.).unwrap();
    let partial = rows.iter().find(|r| r["pci"] == 367.).unwrap();
    assert_eq!(first["rsrp"], -90.);
    assert_eq!(first["rsrq"], -12.);
    assert!(first["sinr"].is_null());
    assert!(missing["rsrp"].is_null());
    assert!(missing["rsrq"].is_null());
    assert!(missing["sinr"].is_null());
    assert_eq!(partial["rsrp"], -98.);
    assert!(partial["rsrq"].is_null());
    assert_eq!(partial["sinr"], 2.);
}
#[test]
fn neighbors_missing_trailing_metric_keep_reported_identity_and_power() {
    let value = radio::parse(
        "tdtech",
        "^MONNC: NR,627264,D8,-720,-96\n^MONNC: LTE,1300,100,-85,-9\n",
    )
    .unwrap();
    let rows = value["neighbors"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let nr = rows.iter().find(|r| r["rat"] == "NR").unwrap();
    assert_eq!(nr["rsrp"], -90.);
    assert_eq!(nr["rsrq"], -12.);
    assert!(nr["sinr"].is_null());
    let lte = rows.iter().find(|r| r["rat"] == "LTE").unwrap();
    assert_eq!(lte["rsrp"], -85.);
    assert!(lte["rssi"].is_null());
}
#[test]
fn query_profiles_are_bounded_and_platform_specific() {
    for (_, models) in catalog::MODELS {
        for model in models.split_whitespace() {
            let p = catalog::identify(model);
            let q = p["queries"].as_array().unwrap();
            assert!(q.len() <= MAX_SAMPLE_QUERIES, "{model}");
            for cmd in q {
                assert!(security::read_only(cmd.as_str().unwrap()), "{model} {cmd}");
            }
            let ambr = q.iter().any(|s| s == "AT+QNWCFG=\"nr5g_ambr\"");
            if p["platform"] == "unisoc" {
                assert!(!ambr);
            }
        }
    }
    assert!(!sms_settings::supported("rm520n-gl"));
    assert!(!sms_settings::supported("mh5000-82"));
    let state = sms_settings::confirmed("^IMSSWITCH: 0,1,0\nOK\n").unwrap();
    assert_eq!(state.command(true), "AT^IMSSWITCH=1,1,0");
    assert!(sms_settings::confirmed("^IMSSWITCH: 0,0,0\nERROR\n").is_none());
    assert!(sms_settings::confirmed("^IMSSWITCH: 0,0\nOK\n").is_none());
}
