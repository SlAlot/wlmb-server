#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::radio;

const SERVING: &str = "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,01,5F1EA15,12,1650,3,5,5,DE10,-100,-12,-68,11\n";
#[test]
fn lte_secondary_keeps_its_measurements_and_snr_is_not_sinr() {
    let raw = format!(
        "{SERVING}+QCAINFO: \"PCC\",1650,100,\"LTE BAND 3\",1,12,-99,-11,-66,21\n+QCAINFO: \"SCC\",2850,100,\"LTE BAND 7\",2,88,-105,-13,-73,9\n+QCAINFO: \"SCC\",2850,100,\"LTE BAND 7\",2,88,-105,-13,-73,9\nOK"
    );
    let v = radio::parse("quectel", &raw).unwrap();
    let rows = v["carriers"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let row = rows.iter().find(|r| r["role"] == "scc").unwrap();
    assert_eq!(row["rsrp"], -105.);
    assert_eq!(row["rsrq"], -13.);
    assert_eq!(row["rssi"], -73.);
    assert_eq!(row["snr"], 9.);
    assert!(row["sinr"].is_null());
    assert_eq!(row["dlBandwidthMHz"], 20.);
    assert_eq!(row["active"], true);
    assert_eq!(row["metricSources"]["rsrp"], "AT+QCAINFO");
}
#[test]
fn short_neighbors_are_not_dropped_or_labeled_as_sinr() {
    let raw = format!(
        "{SERVING}+QENG: \"neighbourcell\",\"LTE\",2850,88,-105,-13,37\n+QENG: \"neighbourcell\",\"LTE\",1650,12,-100,-12,37\nOK"
    );
    let v = radio::parse("quectel", &raw).unwrap();
    let rows = v["neighbors"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["band"], serde_json::Value::Null);
    assert_eq!(rows[0]["bandCandidates"], serde_json::json!(["B7"]));
    assert_eq!(rows[0]["rsrp"], -105.);
    assert_eq!(rows[0]["rsrq"], -13.);
    assert!(rows[0]["sinr"].is_null());
}
#[test]
fn missing_pci_does_not_erase_real_nr_ca_or_copy_primary_signal() {
    let raw = "+QENG: \"servingcell\",\"CONNECT\",\"NR5G-SA\",\"TDD\",460,01,9013B004,299,690E0F,633984,78,12,-80,-5,20\n+QCAINFO: \"PCC\",633984,100M,\"NR5G BAND 78\",1,299\n+QCAINFO: \"SCC\",636664,90MHz,\"NR5G BAND 78\",2,-\n+QCAINFO: \"SCC\",633984,100M,\"NR5G BAND 78\",2,300\nOK";
    let v = radio::parse("quectel", raw).unwrap();
    let rows = v["carriers"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    let primary = rows.iter().find(|r| r["role"] == "pcc").unwrap();
    assert_eq!(primary["rsrp"], -80.);
    for row in rows.iter().filter(|r| r["role"] == "scc") {
        assert!(row["rsrp"].is_null());
        assert!(row["sinr"].is_null());
    }
    assert_eq!(
        rows.iter().find(|r| r["channel"] == 636664.).unwrap()["dlBandwidthMHz"],
        90.
    );
    let malformed = "+QCAINFO: \"SCC\",1650.5,100,\"LTE BAND 3\",2,12\n+QCAINFO: \"SCC\",1650,100,\"LTE BAND 3\",2,12.5\n+QCAINFO: \"SCC\",1650,100,\"LTE BAND 3\",9,12\nOK";
    assert!(
        radio::parse("quectel", malformed).unwrap()["carriers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
