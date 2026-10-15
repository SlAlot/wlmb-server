#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::radio;
const LTE: &str =
    "+CPSI: LTE,Online,460-11,0x1111,1234567,123,EUTRAN-BAND3,1850,5,4,-90,-850,-650,15\n";
#[test]
fn legacy_nsa_is_visible_without_inventing_an_overlapping_band() {
    let raw = format!("{LTE}+CPSI: NR5G,257,629952,-11,-92,153\nOK");
    let v = radio::parse("simcom", &raw).unwrap();
    let rows = v["serving"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let nr = rows.iter().find(|row| row["rat"] == "NR").unwrap();
    assert_eq!(nr["rsrp"], -92.);
    assert_eq!(nr["rsrq"], -11.);
    assert_eq!(nr["snr"], 15.3);
    assert!(nr["sinr"].is_null());
    assert!(nr["band"].is_null());
    assert_eq!(nr["bandCandidates"], serde_json::json!(["n77", "n78"]));
    assert!(rows.iter().all(|row| row["networkMode"] == "5G NSA"));
    let lte = rows.iter().find(|row| row["rat"] == "LTE").unwrap();
    assert_eq!(lte["sinr"], 15.);
    assert_eq!(lte["dlBandwidthMHz"], 20.);
    assert_eq!(lte["ulBandwidthMHz"], 15.);
}
#[test]
fn legacy_sa_keeps_real_identifiers_and_absent_bandwidth() {
    let v = radio::parse(
        "simcom",
        "+CPSI: NR5G,Online,460-01,0x1234,12345678,488,627264,-81,-9,163\nOK",
    )
    .unwrap();
    assert_eq!(v["serving"][0]["pci"], 488.);
    assert_eq!(v["serving"][0]["networkMode"], "5G SA");
    assert_eq!(v["serving"][0]["snr"], 16.3);
    assert!(v["serving"][0]["dlBandwidthMHz"].is_null());
    let v = radio::parse(
        "simcom",
        "+CPSI: NR5G,Online,460-01,0x1234,12345678,488,627264,-810,-90,163\nOK",
    )
    .unwrap();
    assert_eq!(v["serving"][0]["rsrp"], -81.);
    assert_eq!(v["serving"][0]["rsrq"], -9.);
}
#[test]
fn new_nsa_and_sa_do_not_guess_conflicting_snr_units() {
    for raw in [
        "+CPSI: NR5G_NSA,488,NR5G_BAND78,627264,-850,-100,15,1,12\nOK",
        "+CPSI: NR5G_SA,Online,460,01,0x1234,123456,488,NR5G_BAND78,627264,-850,-100,15\nOK",
    ] {
        let v = radio::parse("simcom", raw).unwrap();
        assert_eq!(v["serving"][0]["rsrp"], -85.);
        assert_eq!(v["serving"][0]["rsrq"], -10.);
        assert!(v["serving"][0]["snr"].is_null());
        assert!(v["serving"][0]["sinr"].is_null());
        assert_eq!(v["serving"][0]["snrRaw"], "15");
    }
}
#[test]
fn malformed_identity_offline_and_sentinels_do_not_make_carriers() {
    for raw in [
        "+CPSI: NR5G,257.1,629952,-11,-92,153\nOK",
        "+CPSI: NR5G,257,629952.2,-11,-92,153\nOK",
        "+CPSI: NR5G_NSA,65535,NR5G_BAND78,627264,-850,-100,150\nOK",
        "+CPSI: NO SERVICE,Online\nOK",
        "+CPSI: NR5G_SA,Offline,460-01,1234,12345,488,NR5G_BAND78,627264,-850,-100,150\nOK",
    ] {
        assert!(
            radio::parse("simcom", raw).unwrap()["carriers"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    let raw = LTE.replace(",123,EUTRAN", ",123.5,EUTRAN");
    assert!(
        radio::parse("simcom", &raw).unwrap()["serving"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(radio::parse("generic", "+CSQ: 15.5,0\nOK").unwrap()["rssi"].is_null());
    assert!(radio::parse("generic", "+CSQ: 99,99\nOK").unwrap()["rssi"].is_null());
}
