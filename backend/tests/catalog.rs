#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use std::collections::BTreeSet;
use wlmb_server::{catalog, security};

#[test]
fn inventory_separates_identity_from_protocol_and_acceptance() {
    let inventory = catalog::inventory();
    let mut models = BTreeSet::new();
    for row in inventory["models"].as_array().unwrap() {
        let model = row["model"].as_str().unwrap();
        assert!(models.insert(model));
        assert_eq!(row["hardwareAccepted"], false);
        assert_ne!(row["platform"], "unknown");
        let radio = catalog::radio_capabilities(model);
        for feature in ["neighbors", "carrierAggregation"] {
            assert_eq!(
                row["features"][feature],
                if radio[feature] == true {
                    "implemented-field-dependent"
                } else {
                    "not-exposed-by-parser"
                }
            );
        }
        assert_eq!(
            row["features"]["temperature"] == "implemented-firmware-dependent",
            wlmb_server::temperature::query(model).is_some()
        );
        assert_eq!(
            row["features"]["sessionQos"] == "implemented-firmware-dependent",
            row["queries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|query| query == "AT+CGEQOSRDP")
        );
        for query in row["queries"].as_array().unwrap() {
            assert!(
                security::read_only(query.as_str().unwrap()),
                "{model}: {query}"
            );
        }
        if row["parser"].is_null() {
            assert_eq!(row["features"]["radio"], "pending");
        }
        if !wlmb_server::lock_protocol::band_supported(model) {
            assert_eq!(row["features"]["bandLock"], "pending");
        } else {
            assert_eq!(row["features"]["bandLock"], "implemented-unverified-write");
        }
    }
    assert_eq!(catalog::platform("unlisted-qualcomm"), "unknown");
}

#[test]
fn same_vendor_does_not_imply_same_platform_protocol() {
    assert_eq!(catalog::platform("FM350-GL"), "mediatek");
    assert_eq!(catalog::platform("FM650-CN"), "unisoc");
    assert_eq!(catalog::platform("FM160-CN"), "qualcomm");
    assert_eq!(catalog::platform("NL668"), "lte");
    assert_eq!(catalog::platform("MT5710_CN"), "hisilicon");
    assert_eq!(catalog::platform("MH5000-82M"), "unisoc");
    assert_eq!(catalog::identify("MH5000-82M")["parser"], "huawei-unisoc");
    assert_ne!(
        catalog::identify("RM500U-CN")["parser"],
        catalog::identify("RM500Q-CN")["parser"]
    );
}
#[test]
fn summary_and_pending_protocols_do_not_advertise_nonexistent_ca_or_neighbors() {
    for model in ["NL668", "Air724UG"] {
        let c = catalog::radio_capabilities(model);
        assert_eq!(c["signal"], true);
        assert_eq!(c["neighbors"], true);
        assert_eq!(c["carrierAggregation"], false);
    }
    for model in ["MC7354", "SIMCOM-SIM8200EA-M2", "unlisted-modem"] {
        let c = catalog::radio_capabilities(model);
        assert_eq!(c["signal"], true);
        assert_eq!(c["neighbors"], false);
        assert_eq!(c["carrierAggregation"], false);
    }
    for model in ["SRM825", "N510M", "GM800", "ME3630-W", "MV32-W-A"] {
        let c = catalog::radio_capabilities(model);
        assert_eq!(c["signal"], true);
        assert_eq!(c["neighbors"], false);
        assert_eq!(c["carrierAggregation"], false);
    }
    for model in ["MH5000-82M", "RG801", "FN990A40", "T99W640"] {
        let c = catalog::radio_capabilities(model);
        assert_eq!(c["signal"], true);
        assert_eq!(c["neighbors"], false);
        assert_eq!(c["carrierAggregation"], true);
    }
    assert_eq!(catalog::radio_capabilities("NL678")["neighbors"], true);
    assert_eq!(
        catalog::radio_capabilities("MT5700M-CN")["carrierAggregation"],
        true
    );
    assert_eq!(
        catalog::radio_capabilities("RM520N-GL")["carrierAggregation"],
        true
    );
}

#[test]
fn every_pinned_upstream_transport_profile_has_explicit_identity_and_platform() {
    let upstream: serde_json::Value =
        serde_json::from_str(include_str!("../../docs/qmodem-upstream.json")).unwrap();
    for row in upstream["models"].as_array().unwrap() {
        let model = row["model"].as_str().unwrap();
        let profile = catalog::identify(model);
        assert_ne!(profile["family"], "generic", "missing identity: {model}");
        assert_eq!(
            profile["platform"], row["platform"],
            "platform changed: {model}"
        );
        let family = if row["vendor"] == "huawei" {
            "tdtech"
        } else {
            row["vendor"].as_str().unwrap()
        };
        assert_eq!(profile["family"], family, "vendor mismatch: {model}");
    }
}

#[test]
fn automatic_endpoint_candidates_use_kernel_type_not_modem_or_usb_offset() {
    use serde_json::json;
    use wlmb_server::modem_inventory::at_candidate;
    assert!(at_candidate(&json!({"port":"/dev/wwan0at0"})));
    assert!(at_candidate(
        &json!({"port":"/dev/ttyUSB7","interfaceHint":"Modem AT"})
    ));
    for hint in [
        "Diagnostic AT",
        "GPS AT",
        "AT NMEA",
        "QDSS AT",
        "TRACE AT",
        "DATA",
        "QAT",
    ] {
        assert!(
            !at_candidate(&json!({"port":"/dev/ttyUSB1","interfaceHint":hint})),
            "{hint}"
        );
    }
    assert!(!at_candidate(
        &json!({"port":"/dev/ttyUSB1","productHint":"RM520N-GL"})
    ));
    assert!(!at_candidate(
        &json!({"port":"/dev/ttyS1","interfaceHint":"AT"})
    ));
    assert!(!at_candidate(&json!({"port":"/dev/wwan0mbim0"})));
}
