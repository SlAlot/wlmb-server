#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::{catalog, radio, security};
const MC_STATUS: &str = "!GSTATUS:\nCurrent Time: 2863 Temperature: 31\nBootup Time: 0 Mode: ONLINE\nSystem mode: LTE PS state: Attached\nLTE band: B4 LTE bw: 15 MHz\nLTE Rx chan: 2325 LTE Tx chan: 20325\nRSSI (dBm): -75 Tx Power: 15\nRSRP (dBm): -105 TAC: 3604 (13828)\nRSRQ (dB): -16 Cell ID: 0568CF16 (90754838)\nSINR (dB): 3.6\nOK\n";
#[test]
fn mc73_summary_keeps_metrics_without_fabricating_pci_ul_or_neighbors() {
    let v = radio::parse("sierra-lte", MC_STATUS).unwrap();
    let r = &v["serving"][0];
    assert_eq!(r["channel"], 2325.);
    assert_eq!(r["band"], "B4");
    assert_eq!(r["rssi"], -75.);
    assert_eq!(r["rsrp"], -105.);
    assert_eq!(r["rsrq"], -16.);
    assert_eq!(r["sinr"], 3.6);
    assert_eq!(r["dlBandwidthMHz"], 15.);
    assert!(r["pci"].is_null());
    assert!(r["ulBandwidthMHz"].is_null());
    assert_eq!(v["neighbors"].as_array().unwrap().len(), 0);
    for m in ["MC7354", "MC7355"] {
        assert_eq!(catalog::identify(m)["parser"], "sierra-lte");
    }
}
#[test]
fn mc73_rejects_detached_non_lte_mismatch_and_incomplete_response() {
    for raw in [
        MC_STATUS.replace("Attached", "Detached"),
        MC_STATUS.replace("System mode: LTE", "System mode: WCDMA"),
        MC_STATUS.replace("B4", "B3"),
        MC_STATUS.replace("OK\n", ""),
    ] {
        assert!(
            radio::parse("sierra-lte", &raw).unwrap()["serving"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    let duplicate = MC_STATUS.replace("SINR (dB): 3.6", "SINR (dB): 3.6\nSINR (dB): 99");
    assert!(radio::parse("sierra-lte", &duplicate).unwrap()["serving"][0]["sinr"].is_null());
}
const LTE: &str = "!LTEINFO:\nServing: EARFCN MCC MNC TAC CID Bd D U SNR PCI RSRQ RSRP RSSI RXLV\n1800 460 01 42 00FF0012 3 5 4 12 90 -8 -830 -66 40\nIntraFreq: PCI RSRQ RSRP RSSI RXLV\n91 -10 -900 -70 20\nInterFreq: EARFCN ThresholdLow ThresholdHi Priority PCI RSRQ RSRP RSSI RXLV\n40000 5 10 7 22 -12 -95 -74 20\nCA SCell: EARFCN SCID Bd ST D U Mdl Mul PCI RSRP RSSI SINR\n6300 1 20 2 3 2 2 1 55 -920 -73 175\n2850 2 7 1 5 5 2 1 56 -990 -76 100\nOK\n";
fn nr(id: &str, channel: u32, index: u32) -> String {
    format!(
        "NR5G Cell ID: {id}\nNR5G MCC-MNC: 460-01\nNR5G band: n78 NR5G Carrier ID: {index}\nNR5G dl bw: 100 MHz NR5G ul bw: 100 MHz\nNR5G Rx chan: {channel}\nNR5G(sub6) RxM RSSI (dBm): -68.5 NR5G(sub6) RxD RSSI (dBm): -72.2\n"
    )
}
fn response(mode: &str, blocks: &str) -> String {
    format!(
        "!NRINFO:\nConnectivity Mode: {mode}\n{blocks}\nNR5G RSRP (dBm): -84 NR5G RSRQ (dB): -9\nNR5G SINR (dB): 21\nOK\n"
    )
}
#[test]
fn lte_tables_keep_real_neighbors_units_and_secondary_state() {
    let v = radio::parse("sierra-em91", LTE).unwrap();
    assert_eq!(v["serving"][0]["rsrp"], -83.);
    assert_eq!(v["serving"][0]["dlBandwidthMHz"], 20.);
    assert_eq!(v["serving"][0]["ulBandwidthMHz"], 15.);
    assert_eq!(v["serving"][0]["rssnr"], 12.);
    assert!(
        v["serving"][0]["sinr"].is_null(),
        "RSSNR must not be silently called SINR"
    );
    let ca = v["carriers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["channel"] == 6300.)
        .unwrap();
    assert_eq!(ca["sinr"], 15.);
    assert_eq!(ca["active"], true);
    assert!(ca["rsrq"].is_null());
    assert_eq!(v["neighbors"].as_array().unwrap().len(), 2);
    assert!(
        v["neighbors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["channel"] == 1800. && r["pci"] == 91.)
    );
    assert!(
        v["carriers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["channel"] == 2850. && r["active"] == false)
    );
}
#[test]
fn em91_pci_and_em92_global_cell_id_are_not_interchangeable() {
    let a = radio::parse("sierra-em91", &response("SA", &nr("488", 627264, 0))).unwrap();
    assert_eq!(a["serving"][0]["pci"], 488.);
    assert_eq!(a["serving"][0]["rsrp"], -84.);
    let b = radio::parse(
        "sierra-em92",
        &response("SA", &nr("ef4c717d7 (64236230615)", 627264, 0)),
    )
    .unwrap();
    assert!(b["serving"][0]["pci"].is_null());
    assert_eq!(b["serving"][0]["cellId"], "ef4c717d7");
    assert_eq!(b["serving"][0]["rssi"], -68.5);
}
#[test]
fn two_nr_carriers_do_not_gain_fake_pcc_roles_or_duplicated_summary_signal() {
    let raw = response("SA", &(nr("488", 627264, 0) + &nr("489", 636664, 1)));
    let v = radio::parse("sierra-em91", &raw).unwrap();
    assert_eq!(v["carriers"].as_array().unwrap().len(), 2);
    for row in v["carriers"].as_array().unwrap() {
        assert_eq!(row["roleKnown"], false);
        assert!(row["rsrp"].is_null());
    }
    assert_eq!(v["serving"][0]["rsrp"], -84.);
    assert!(v["serving"][0]["channel"].is_null());
    assert_eq!(v["serving"][0]["measurementScope"], "unbound-NR-summary");
}
#[test]
fn stale_lte_inventory_cannot_turn_a_current_sa_cell_into_nsa_ca() {
    let v = radio::parse(
        "sierra-em91",
        &(LTE.to_owned() + &response("SA", &nr("488", 627264, 0))),
    )
    .unwrap();
    assert_eq!(v["carriers"].as_array().unwrap().len(), 1);
    assert_eq!(v["serving"][0]["networkMode"], "5G SA");
    let v = radio::parse(
        "sierra-em91",
        &(LTE.to_owned() + &response("NSA", &nr("---", 627264, 0))),
    )
    .unwrap();
    assert!(
        v["carriers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["rat"] == "LTE")
    );
    assert_eq!(v["serving"][0]["networkMode"], "5G NSA");
}
#[test]
fn no_service_malformed_rows_and_unknown_fields_do_not_create_cells() {
    let v=radio::parse("sierra-em91","!NRINFO:\nConnectivity Mode: ---\nNR5G Cell ID: ---\nNR5G band: ---\nNR5G Rx chan: ---\nOK\n!LTEINFO: Unavailable\nOK").unwrap();
    assert!(v["carriers"].as_array().unwrap().is_empty());
    let v = radio::parse(
        "sierra-em91",
        "!LTEINFO:\nIntraFreq: PCI RSRQ RSRP RSSI RXLV\n90 -8 -83 -66 30\nOK",
    )
    .unwrap();
    assert!(v["neighbors"].as_array().unwrap().is_empty());
    let raw = response("SA", &nr("488", 627264, 0)).replace("627264", "627264.5");
    assert!(
        radio::parse("sierra-em91", &raw).unwrap()["carriers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn profile_selects_only_documented_em9_protocols_with_safe_queries() {
    assert_eq!(catalog::identify("EM9190")["parser"], "sierra-em91");
    assert_eq!(catalog::identify("EM9291")["parser"], "sierra-em92");
    assert_eq!(catalog::identify("MC7354")["parser"], "sierra-lte");
    for c in ["AT!NRINFO?", "AT!LTEINFO?"] {
        assert!(security::read_only(c));
    }
    assert!(!security::read_only("AT!LTECA=0"));
}
