#![allow(clippy::unwrap_used)]
// 对照各平台实际命令参数，确保界面不会向所有模组要求同一组字段。
use serde_json::json;
use wlmb_server::{band_inventory, lock_protocol};

#[test]
fn fields_follow_protocol_not_transport() {
    for (model, nr_band, nr_scs, lte_band) in [
        ("mt5700m-cn", true, true, true),
        ("rm500u-cn", false, false, false),
        ("rm520n-gl", true, true, false),
        ("fm350-gl", true, true, false),
        ("sim8262e-m2", true, true, false),
    ] {
        let fields = lock_protocol::cell_fields(model);
        assert_eq!(fields["NR"]["bandRequired"], nr_band, "{model}");
        assert_eq!(fields["NR"]["scsRequired"], nr_scs, "{model}");
        assert_eq!(fields["LTE"]["bandRequired"], lte_band, "{model}");
        assert_eq!(fields["LTE"]["scsRequired"], false);
        let preset = band_inventory::presets(model);
        assert_eq!(preset["cellFields"], fields);
        for rat in ["NR", "LTE"] {
            for row in preset["cellRanges"][rat].as_array().unwrap() {
                assert!(preset[rat].as_array().unwrap().contains(&row["band"]));
                assert!(row["first"].as_i64().unwrap() <= row["last"].as_i64().unwrap());
            }
        }
    }
}

#[test]
fn simple_protocol_accepts_pair_without_fabricated_fields() {
    let op = json!({"rat":"NR","mode":"cell","entries":[{"pci":0,"arfcn":627264}]});
    assert_eq!(
        lock_protocol::command("rm500u-cn", &op).unwrap(),
        "AT+QNWLOCK=\"common/5g\",1,627264,0"
    );
    assert!(lock_protocol::command("mt5700m-cn", &op).is_err());
    assert!(lock_protocol::command("rm520n-gl", &op).is_err());
}

#[test]
fn negotiated_ranges_are_rebuilt_instead_of_reusing_unique_guess() {
    let mut preset = band_inventory::presets("mt5700m-cn");
    preset["NR"] = json!([77, 78]);
    band_inventory::cell_metadata(&mut preset, "mt5700m-cn");
    assert_eq!(preset["cellRanges"]["NR"].as_array().unwrap().len(), 2);
    preset["NR"] = json!([78]);
    band_inventory::cell_metadata(&mut preset, "mt5700m-cn");
    assert_eq!(preset["cellRanges"]["NR"].as_array().unwrap().len(), 1);
}

#[test]
fn all_catalog_models_expose_the_same_requirements_on_each_transport() {
    let inventory = wlmb_server::catalog::inventory();
    for row in inventory["models"].as_array().unwrap() {
        let model = row["model"].as_str().unwrap();
        let preset = band_inventory::presets(model);
        assert_eq!(
            preset["cellFields"],
            lock_protocol::cell_fields(model),
            "{model}"
        );
        assert_eq!(
            preset["cellFields"]["NR"]["scsRequired"],
            lock_protocol::scs_required(model)
        );
        assert_eq!(preset["cellFields"]["LTE"]["scsRequired"], false);
    }
}
