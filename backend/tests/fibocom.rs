#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::{catalog, radio};
const HEX: &str = "+GTCCINFO:\r\nNR service cell:\r\n1,9,460,01,ABC,12345,992F0,1E8,5078,12,79,0,76,69\r\n2,4,460,01,ABC,23456,708,5B,0,0,51,22\r\nOK\r\n+GTCAINFO:\r\nPCC: 5078,1E8,992F0,12,0,0,0,0,0\r\nSCC 1: 2,0,5078,1E9,9B700,12,0,0,0,0,0,0\r\nOK\r\n";
#[test]
fn platform_drives_address_base_even_when_all_digits() {
    let v = radio::parse("fibocom-qualcomm", HEX).unwrap();
    let main = &v["serving"][0];
    assert_eq!(main["channel"], 627440.);
    assert_eq!(main["pci"], 488.);
    assert_eq!(main["rsrp"], -81.);
    assert_eq!(main["rsrq"], -9.);
    assert_eq!(main["sinr"], 16.);
    assert_eq!(main["dlBandwidthMHz"], 100.);
    assert!(main["ulBandwidthMHz"].is_null());
    assert_eq!(v["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(v["carriers"][1]["channel"], 636672.);
    assert!(v["carriers"][1]["rsrp"].is_null());
    assert!(v["carriers"][1]["ulBandwidthMHz"].is_null());
    assert_eq!(v["neighbors"][0]["channel"], 1800.);
    assert_eq!(v["neighbors"][0]["pci"], 91.);
}
#[test]
fn mixed_lte_nr_and_invalid_quantized_values_do_not_invent_signal() {
    let raw = "+GTCCINFO:\n1,4,460,01,AB,12345,708,5A,103,5,0,0,57,24\n1,9,460,01,AB,12345,992F0,1E8,5078,12,255,0,255,255\nOK\n";
    let v = radio::parse("fibocom-qualcomm", raw).unwrap();
    assert_eq!(v["serving"].as_array().unwrap().len(), 2);
    assert!(
        v["serving"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["networkMode"] == "5G NSA")
    );
    let nr = v["serving"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["rat"] == "NR")
        .unwrap();
    for metric in ["rsrp", "rsrq", "sinr", "rssi"] {
        assert!(nr[metric].is_null());
    }
    assert_eq!(
        radio::parse("fibocom-qualcomm", &raw.replace("992F0", "not-hex")).unwrap()["serving"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn exact_family_models_use_corresponding_protocol_and_not_vendor_default() {
    for model in [
        "fm150-ae",
        "fm160-cn",
        "fm160-eau",
        "fm160-na",
        "fm170-eau",
        "fm190w-gl",
    ] {
        assert_eq!(catalog::identify(model)["parser"], "fibocom-qualcomm");
    }
    for model in ["fm350-gl", "fm350-gl-00", "fm350r-gl", "rw350r-gl"] {
        assert_eq!(catalog::identify(model)["parser"], "fibocom-fm350");
    }
    assert_eq!(catalog::identify("fm650-cn")["parser"], "fibocom-hex");
    assert_eq!(catalog::identify("nl668")["parser"], "fibocom-nl668");
}
#[test]
fn mediatek_decimal_addresses_and_widths_do_not_inherit_qualcomm_encoding() {
    let raw = "+GTCCINFO:\n1,9,460,01,1120,23456,627440,488,5078,500,0,0,76,69\nOK\n";
    let v = radio::parse("fibocom-fm350", raw).unwrap();
    assert_eq!(v["serving"][0]["channel"], 627440.);
    assert_eq!(v["serving"][0]["pci"], 488.);
    assert_eq!(v["serving"][0]["dlBandwidthMHz"], 100.);
    assert!(v["serving"][0]["sinr"].is_null());
    let wrong = radio::parse("fibocom-qualcomm", raw).unwrap();
    assert!(wrong["serving"].as_array().unwrap().is_empty());
}
#[test]
fn truncated_or_failed_optional_blocks_do_not_create_carriers() {
    let main = HEX.split("+GTCAINFO:").next().unwrap();
    for end in ["", "ERROR\n", "+CME ERROR: 3\n", "+CMS ERROR: 500\n"] {
        let raw = format!("{main}+GTCAINFO:\nSCC 1: 2,0,5078,1E9,9B700,12,0,0,0,0,0,0\n{end}");
        let v = radio::parse("fibocom-qualcomm", &raw).unwrap();
        assert_eq!(v["serving"].as_array().unwrap().len(), 1);
        assert_eq!(v["carriers"].as_array().unwrap().len(), 1);
    }
    assert!(
        radio::parse("fibocom-qualcomm", &main.replace("OK", "ERROR")).unwrap()["serving"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
