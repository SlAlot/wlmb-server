#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::{catalog, radio};
const HUAWEI: &str = "AT^MONSC\n^MONSC: NR,460,01,627264,1,8115E5002,1E8,811802,-81,-10,18\nOK\n";
const GT: &str = "+GTCCINFO:\n1,9,460,01,ABC,12345,992F0,1E8,5078,12,79,0,76,69\n2,4,460,01,ABC,23456,708,5B,0,0,51,22\nOK\n";
const DEBUG: &str = "RAT: NR5G_SA\nmcc:460,mnc:01\nchannel:627264 pci:488\nnr_cell_id:8115E5002\nnr_band:78 nr_band_width:100 MHz\nnr_snr:18 rsrq:-10 rsrp:-81 nr_rssi:-65\nnr_tac:811802\nnr_scc1:\nnr_band:78 nr_band_width:100 MHz\nOK\n";
const TELIT: &str = "#CAINFOEXT: 2,NR5G_SA\nPCC-NR, BandClass:269, BW:100 MHz, CH:627264, PCI:488, RSRP:-81, RSRQ:-10, RSSI:-65, SINR:190, TAC:811802\nSCC0-NR, BandClass:269, BW:100 MHz, RX_CH:636664, PCI:489,\n RSRP:-92, RSRQ:-13, SINR:150\nOK\n";
const NEOWAY: &str =
    "+NETDMSGEX: \"NR to 5GCN\",\"460+01\",\"78\",627264,12345,488,-810,-100,180\nOK\n";
const GM800: &str = "+ZCELLINFO: NR5G,tac:811802,cell_id:8115E5002,pci:488,band:78,freq:627264,mcc:460,mnc:01,rsrp:-81,rsrq:-10,rssi:-65,sinr:18\nOK\n";
const ME3630: &str = "+ZCELLINFO: 8118,cellid:12345,pci:488,band:3\nOK\n+CESQ:99,99,255,255,20,60\nOK\n+CSQ:20,99\nOK\n";
const QUECTEL: &str = "+QENG: \"servingcell\",\"NOCONN\",\"NR5G-SA\",\"TDD\",460,01,8115E5002,488,811802,627264,78,12,-81,-10,18\nOK\n";
fn meig() -> String {
    let mut p = vec!["0"; 17];
    for (i, v) in [
        (0, "5G"),
        (1, "TDD"),
        (2, "460"),
        (3, "01"),
        (4, "8115E5002"),
        (5, "488"),
        (6, "811802"),
        (7, "78"),
        (8, "100"),
        (9, "1"),
        (14, "-81"),
        (15, "-10"),
        (16, "180"),
    ] {
        p[i] = v;
    }
    format!("^CELLINFO:{}\nOK\n", p.join(","))
}
#[test]
fn all_remaining_21_models_parse_real_protocol_fields_not_a_generic_model_list() {
    let meig = meig();
    for (models, fixture) in [
        ("fm650-cn l610-eu nl678", GT),
        ("mh5000-82 mh5000-82m", HUAWEI),
        ("rg801", QUECTEL),
        ("srm815 srm825 srm825n srm821", meig.as_str()),
        ("t77w968 t99w175 t99w368 t99w373 t99w640", DEBUG),
        ("fn990a40", TELIT),
        ("gm800", GM800),
        ("me3630-w", ME3630),
        ("mv32-w-a mv32-w-b", DEBUG),
        ("n510m", NEOWAY),
    ] {
        for model in models.split_whitespace() {
            let profile = catalog::identify(model);
            let parser = profile["parser"].as_str().unwrap();
            let r = radio::parse(parser, fixture).unwrap();
            let row = &r["serving"][0];
            assert!(!row["rsrp"].is_null(), "{model}: {r}");
            assert!(!row["rsrq"].is_null(), "{model}: {r}");
            assert_ne!(parser, "generic");
        }
    }
    for model in catalog::inventory()["models"].as_array().unwrap() {
        assert!(model["parser"].is_string(), "{}", model["model"]);
    }
}
#[test]
fn debug_subcarrier_identity_and_signal_are_not_borrowed_from_primary() {
    let r = radio::parse("foxconn-plus", DEBUG).unwrap();
    assert_eq!(r["serving"][0]["sinr"], 18.);
    assert_eq!(r["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(r["carriers"][1]["band"], "n78");
    assert!(r["carriers"][1]["pci"].is_null());
    assert!(r["carriers"][1]["rsrp"].is_null());
    let thales = radio::parse("thales-debug", DEBUG).unwrap();
    assert_eq!(thales["carriers"].as_array().unwrap().len(), 1);
    let lte = DEBUG
        .split("nr_scc1:")
        .next()
        .unwrap()
        .replace("NR5G_SA", "LTE+NR")
        .replace("nr_", "lte_")
        + "OK\n";
    let r = radio::parse("thales-debug", &lte.replace("rsrp:", "lte_rsrp:")).unwrap();
    assert_eq!(
        r["serving"].as_array().unwrap().len(),
        0,
        "NR ARFCN is not a valid LTE channel"
    );
    let r = radio::parse(
        "thales-debug",
        &lte.replace("627264", "1800").replace("rsrp:", "lte_rsrp:"),
    )
    .unwrap();
    assert_eq!(r["serving"][0]["rsrp"], -81.);
    assert_eq!(r["serving"][0]["networkMode"], "5G NSA");
}
#[test]
fn telit_reports_component_metrics_converts_band_class_and_sinr_without_fake_count() {
    let r = radio::parse("telit-cainfoext", TELIT).unwrap();
    assert_eq!(r["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(r["serving"][0]["band"], "n78");
    assert_eq!(r["serving"][0]["sinr"], 18.);
    assert_eq!(r["carriers"][1]["rsrp"], -92.);
    assert_eq!(r["inventoryIncomplete"], false);
    let r = radio::parse("telit-cainfoext", &TELIT.replace("2,NR5G_SA", "3,NR5G_SA")).unwrap();
    assert_eq!(r["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(r["inventoryIncomplete"], true);
    let lte = TELIT
        .replace("269", "122")
        .replace("CH:627264", "CH:1800")
        .replace("RX_CH:636664", "RX_CH:1850")
        .replace("BW:100 MHz", "DL_BW:5");
    let r = radio::parse("telit-cainfoext", &lte).unwrap();
    assert_eq!(r["serving"][0]["band"], "B3");
    assert_eq!(r["serving"][0]["dlBandwidthMHz"], 20.);
}
#[test]
fn csv_signal_only_schemas_do_not_invent_arfcn_gnb_cellid_or_nsa_pci() {
    let r = radio::parse("meig-cellinfo", &meig()).unwrap();
    assert_eq!(r["serving"][0]["sinr"], 18.);
    assert!(r["serving"][0]["channel"].is_null());
    assert!(r["carriers"].as_array().unwrap().is_empty());
    let mut p = vec!["0"; 38];
    for (i, v) in [
        (0, "LTE-NR"),
        (2, "460"),
        (3, "01"),
        (5, "100"),
        (7, "12345"),
        (8, "123"),
        (9, "3"),
        (10, "100"),
        (14, "-90"),
        (15, "-12"),
        (16, "100"),
        (29, "-85"),
        (30, "-10"),
        (31, "150"),
        (32, "78"),
        (34, "100"),
    ] {
        p[i] = v;
    }
    let r = radio::parse("meig-cellinfo", &format!("^CELLINFO:{}\nOK\n", p.join(","))).unwrap();
    assert_eq!(r["secondaryMeasurement"]["rsrp"], -85.);
    assert!(r["secondaryMeasurement"]["pci"].is_null());
    let r = radio::parse("neoway-netdmsgex", NEOWAY).unwrap();
    assert_eq!(r["serving"][0]["rsrp"], -81.);
    assert!(r["serving"][0]["cellId"].is_null());
    assert_eq!(r["serving"][0]["gnbId"], "12345");
    let r = radio::parse(
        "neoway-netdmsgex",
        &NEOWAY.replace("NR to 5GCN", "NR to EPS"),
    )
    .unwrap();
    assert_eq!(r["serving"][0]["networkMode"], "5G NSA");
}
#[test]
fn gosuncn_never_imports_upstreams_hardcoded_neighbor_examples_or_saturated_signal() {
    let r = radio::parse("gosuncn-lte", ME3630).unwrap();
    assert!(r["serving"][0]["channel"].is_null());
    assert_eq!(r["serving"][0]["rssi"], -73.);
    assert!(r["neighbors"].as_array().unwrap().is_empty());
    let r = radio::parse(
        "gosuncn-lte",
        &ME3630
            .replace(",20,60", ",255,97")
            .replace("+CSQ:20", "+CSQ:31"),
    )
    .unwrap();
    for key in ["rsrp", "rsrq", "rssi"] {
        assert!(r["serving"][0][key].is_null());
    }
    let nsa = "+ZCELLINFO: LTE,tac:123,cellid:456,pci:100,band:3,freq:1800,rsrp:-90,rsrq:-12,sinr:10\nnarfcn:627264,nr5g_pci:488,cell_id:999,band:78,5g_rsrp:-81,5g_rsrq:-10,5g_sinr:18\nOK\n";
    let r = radio::parse("gosuncn-qualcomm", nsa).unwrap();
    assert_eq!(r["serving"].as_array().unwrap().len(), 2);
    assert_eq!(r["serving"][1]["rsrp"], -81.);
    assert!(r["carriers"].as_array().unwrap().is_empty());
}
#[test]
fn incomplete_error_duplicate_and_injected_wire_replies_fail_closed() {
    for (parser, raw) in [
        ("meig-cellinfo", meig()),
        ("neoway-netdmsgex", NEOWAY.into()),
        ("foxconn-plus", DEBUG.into()),
        ("telit-cainfoext", TELIT.into()),
        ("gosuncn-qualcomm", GM800.into()),
        ("huawei-unisoc", HUAWEI.into()),
    ] {
        for end in ["", "ERROR\n", "+CME ERROR:4\nOK\n"] {
            let r = radio::parse(parser, &raw.replace("OK\n", end)).unwrap();
            assert!(r["serving"].as_array().unwrap().is_empty(), "{parser}: {r}");
        }
        let r = radio::parse(
            parser,
            &format!(
                "{}AT+COPS?\n+COPS:0,2,\"46001\"\nOK\n",
                raw.replace("OK\n", "")
            ),
        )
        .unwrap();
        assert!(
            r["serving"].as_array().unwrap().is_empty(),
            "A later command cannot complete {parser}"
        );
    }
    assert!(radio::parse("foxconn-plus",&DEBUG.replace("pci:488","pci:488 pci:489")).unwrap()["serving"].as_array().unwrap().is_empty());
    assert!(radio::parse("foxconn-plus",&DEBUG.replace("rsrp:-81","rsrp:-81 junk")).unwrap()["serving"][0]["rsrp"].is_null());
    assert!(
        radio::parse("telit-cainfoext", &TELIT.replace("269", "999")).unwrap()["serving"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
