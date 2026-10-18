#![allow(clippy::unwrap_used)]
// 参考 QModem 的公开数据结构，全部回复与小区标识均为合成测试数据。
use serde_json::{Value, json};
use wlmb_server::qmodem_info::{
    attach_carriers, attach_neighbors, neighbor_query_safe, retain_inventory,
};

fn snapshot() -> Value {
    json!({"serving":[{"rat":"NR","role":"pcc","channel":627264.0,"pci":214.0,"band":"n78"}],
        "carriers":[{"rat":"NR","role":"pcc","channel":627264.0,"pci":214.0,"band":"n78"}],
        "neighbors":[],"carrierState":{"status":"serving-only"}})
}
fn carriers() -> Value {
    json!({"current_band":{"status":"ok","cells":[
        {"role":"pcc","rat":"NR","channel":"627264","pci":"214","band":"78","dl_bandwidth":"100"},
        {"role":"scc","rat":"NR","channel":"636664","pci":"215","band":"78","dl_bandwidth":"80"}
    ]}})
}
#[test]
fn carrier_inventory_is_bound_and_does_not_borrow_primary_signal() {
    let mut state = snapshot();
    let mut reply = carriers();
    reply["current_band"]["cells"][1]["imei"] = json!("synthetic-private-field");
    attach_carriers(&reply, &mut state, 2000).unwrap();
    assert_eq!(state["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(state["carriers"][0]["role"], "pcc");
    assert_eq!(state["carriers"][1]["band"], "n78");
    assert_eq!(state["carriers"][1]["dlBandwidthMHz"], 80.);
    assert!(state["carriers"][1]["rsrp"].is_null());
    assert_eq!(state["carrierState"]["observedAtMs"], 2000);
    assert_eq!(state["carrierState"]["inventoryComplete"], true);
    assert!(!state.to_string().contains("synthetic-private-field"));
    reply["current_band"]["cells"].as_array_mut().unwrap().pop();
    attach_carriers(&reply, &mut state, 3000).unwrap();
    assert_eq!(state["carriers"].as_array().unwrap().len(), 1);
}
#[test]
fn carrier_errors_are_atomic_and_reject_changed_or_unbound_inventory() {
    let original = snapshot();
    for invalid in [
        json!({}),
        json!({"current_band":{"status":"unsupported","cells":[]}}),
        json!({"current_band":{"status":"not_registered","cells":[]}}),
        json!({"current_band":{"status":"ok","cells":[]}}),
        json!({"current_band":{"status":"ok","cells":[null]}}),
        json!({"current_band":{"status":"ok","cells":[
            {"role":"scc","rat":"NR","channel":"636664","pci":"215","band":"78"}]}}),
        json!({"current_band":{"status":"ok","cells":vec![json!({});65]}}),
    ] {
        let mut state = original.clone();
        assert!(attach_carriers(&invalid, &mut state, 3000).is_err());
        assert_eq!(state, original);
    }
    for (field, value) in [
        ("pci", "215"),
        ("pci", "9999"),
        ("pci", "214.5"),
        ("band", "12"),
        ("channel", "630000"),
        ("channel", "78"),
        ("role", "unknown"),
    ] {
        let mut reply = carriers();
        reply["current_band"]["cells"][0][field] = json!(value);
        let mut state = original.clone();
        assert!(attach_carriers(&reply, &mut state, 3000).is_err());
        assert_eq!(state, original);
    }
    let mut reply = carriers();
    let mut conflicting = reply["current_band"]["cells"][1].clone();
    conflicting["dl_bandwidth"] = json!("60");
    reply["current_band"]["cells"]
        .as_array_mut()
        .unwrap()
        .push(conflicting);
    assert!(attach_carriers(&reply, &mut original.clone(), 3000).is_err());
}
#[test]
fn nsa_inventory_keeps_rat_identity_and_orders_primary_before_secondary() {
    let mut state = snapshot();
    state["serving"]
        .as_array_mut()
        .unwrap()
        .push(json!({"rat":"LTE","channel":1650.0,"pci":40.0}));
    let mut reply = carriers();
    reply["current_band"]["cells"][0]["role"] = json!("nsa");
    reply["current_band"]["cells"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"pcc","rat":"LTE","channel":"1650","pci":"40","band":"3"}));
    attach_carriers(&reply, &mut state, 1000).unwrap();
    assert_eq!(state["carriers"][0]["rat"], "LTE");
    assert_eq!(state["carriers"][1]["role"], "nsa");
    assert_eq!(state["carriers"][2]["role"], "scc");
}
#[test]
fn neighbor_columns_support_legacy_and_correct_lte_without_guessing_nr_encodings() {
    let mut state = snapshot();
    let reply = json!({"neighborcell":{"NR":[
        {"arfcn":"627264","pci":"30","band":"78","rsrp":"-95","rsrq":"-13","sinr":"5"},
        {"arfcn":"627264","pci":"31","ss_rsrp":"54","ss_sinr":"100","imei":"synthetic-private-field"}
    ],"LTE":[
        {"neighbourcell":"neighbourcell intra","arfcn":"1650","pci":"40","rsrp":"-12","rsrq":"-97"},
        {"neighbourcell":"neighbourcell inter","arfcn":"1650","pci":"41","rsrp":"-98","rsrq":"-13"}
    ],"lockcell_status":{"secret":"synthetic-private-field"}}});
    attach_neighbors(&reply, &mut state, 4000).unwrap();
    let rows = state["neighbors"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let old = rows
        .iter()
        .find(|r| r["rat"] == "LTE" && r["pci"] == 40.)
        .unwrap();
    assert_eq!(old["rsrp"], -97.);
    assert_eq!(old["rsrq"], -12.);
    assert_eq!(old["metricEncoding"], "legacy-lte-columns-corrected");
    let current = rows
        .iter()
        .find(|r| r["rat"] == "LTE" && r["pci"] == 41.)
        .unwrap();
    assert_eq!(current["rsrp"], -98.);
    assert_eq!(current["metricEncoding"], "reported");
    let encoded = rows
        .iter()
        .find(|r| r["rat"] == "NR" && r["pci"] == 31.)
        .unwrap();
    assert!(encoded["rsrp"].is_null());
    assert!(encoded["sinr"].is_null());
    assert_eq!(encoded["encodedMetricsUnresolved"], true);
    assert_eq!(state["neighborState"]["observedAtMs"], 4000);
    assert!(!state.to_string().contains("synthetic-private-field"));
}
#[test]
fn duplicate_serving_example_and_conflicting_neighbors_are_not_published() {
    let mut state = snapshot();
    let good = json!({"arfcn":"627264","pci":"30","rsrp":"-95"});
    let mut conflict = good.clone();
    conflict["rsrp"] = json!("-99");
    let reply = json!({"neighborcell":{"LTE":[],"NR":[
        good,good,conflict,{"arfcn":"627264","pci":"214","rsrp":"-80"},
        {"neighbourcell":"ex1","arfcn":"627264","pci":"31"},
        {"arfcn":"627264","pci":"32.5"},{"arfcn":"9999999","pci":"33"},
        {"arfcn":"627264","pci":"34","rsrp":"-100"}
    ]}});
    attach_neighbors(&reply, &mut state, 5000).unwrap();
    assert_eq!(state["neighbors"].as_array().unwrap().len(), 1);
    assert_eq!(state["neighbors"][0]["pci"], 34.);
    assert!(state["neighborState"]["discarded"].as_u64().unwrap() >= 5);
}
#[test]
fn empty_neighbor_reply_is_distinct_from_missing_or_malformed_reply() {
    let mut state = snapshot();
    attach_neighbors(&json!({"neighborcell":{"NR":[],"LTE":[]}}), &mut state, 1).unwrap();
    assert_eq!(state["neighborState"]["status"], "empty");
    for reply in [
        json!({}),
        json!({"neighborcell":{}}),
        json!({"neighborcell":{"NR":[],"LTE":"invalid"}}),
        json!({"neighborcell":{"NR":vec![json!({});513]}}),
        json!({"neighborcell":{"NR":[{"arfcn":"627264","pci":"9999"}]}}),
        json!({"neighborcell":{"NR":[{"neighbourcell":"ex1","arfcn":"627264","pci":"30"}]}}),
    ] {
        let original = state.clone();
        assert!(attach_neighbors(&reply, &mut state, 2).is_err());
        assert_eq!(state, original);
    }
}
#[test]
fn automatic_neighbor_queries_exclude_active_and_example_implementations() {
    for vendor in ["quectel", "fibocom", "openluat", "QUECTEL"] {
        assert!(neighbor_query_safe(&json!({"manufacturer":vendor})));
    }
    assert!(neighbor_query_safe(
        &json!({"profile":{"parser":"quectel"}})
    ));
    for vendor in ["simcom", "gosuncn", "unknown"] {
        assert!(!neighbor_query_safe(
            &json!({"manufacturer":vendor,"profile":{"parser":"quectel"}})
        ));
    }
    assert!(!neighbor_query_safe(&json!({})));
}

#[test]
fn optional_failures_keep_inventory_only_for_same_serving_identity_without_renewing_time() {
    let mut previous = snapshot();
    attach_carriers(&carriers(), &mut previous, 1000).unwrap();
    attach_neighbors(
        &json!({"neighborcell":{"NR":[{"arfcn":"627264","pci":"30","rsrp":"-95"}]}}),
        &mut previous,
        1000,
    )
    .unwrap();
    let mut current = snapshot();
    current["carrierState"] =
        json!({"status":"serving-only","queryStatus":"error","code":"QMODEM_CARRIERS_CHANGED"});
    current["neighborState"] =
        json!({"status":"unavailable","queryStatus":"error","code":"SYSTEM_UNAVAILABLE"});
    retain_inventory(&mut current, &previous);
    assert_eq!(current["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(current["neighbors"].as_array().unwrap().len(), 1);
    assert_eq!(current["neighborState"]["observedAtMs"], 1000);
    assert_eq!(current["neighbors"][0]["observedAtMs"], 1000);
    assert_eq!(current["neighborState"]["queryStatus"], "error");
    assert_eq!(current["carrierState"]["code"], "QMODEM_CARRIERS_CHANGED");
    let mut changed = snapshot();
    changed["serving"][0]["pci"] = json!(215.);
    changed["neighborState"] = json!({"queryStatus":"error"});
    retain_inventory(&mut changed, &previous);
    assert!(changed["neighbors"].as_array().unwrap().is_empty());
    let mut empty = snapshot();
    attach_neighbors(
        &json!({"neighborcell":{"NR":[],"LTE":[]}}),
        &mut empty,
        2000,
    )
    .unwrap();
    retain_inventory(&mut empty, &previous);
    assert!(empty["neighbors"].as_array().unwrap().is_empty());
}

#[test]
fn repeated_radio_identifiers_on_other_networks_do_not_retain_old_inventory() {
    let mut previous = snapshot();
    attach_neighbors(
        &json!({"neighborcell":{"NR":[
        {"arfcn":"627264","pci":"30","rsrp":"-95"}]}}),
        &mut previous,
        1000,
    )
    .unwrap();
    for field in ["cellId", "tac", "mcc", "mnc"] {
        let mut changed = snapshot();
        changed["serving"][0][field] = json!("changed");
        changed["neighborState"] = json!({"queryStatus":"pending"});
        retain_inventory(&mut changed, &previous);
        assert!(
            changed["neighbors"].as_array().unwrap().is_empty(),
            "{field}"
        );
    }
}
