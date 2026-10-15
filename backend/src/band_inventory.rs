//! 频段事实参考QModem公开型号资料，来源记录在docs/model-bands.json；不等于锁定协议通过。
use serde_json::{Value, json};
use std::sync::LazyLock;
static FACTS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../docs/model-bands.json"))
        .unwrap_or_else(|_| json!({"profiles":[]}))
});
pub fn presets(model: &str) -> Value {
    let key = crate::catalog::normalize(model);
    let key = crate::catalog::identify(&key)["model"]
        .as_str()
        .unwrap_or(&key)
        .to_owned();
    let rows: Vec<_> = FACTS["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["model"] == key)
        .collect();
    let bands = |field: &str| {
        let lists: Vec<_> = rows
            .iter()
            .filter_map(|r| r[field].as_array())
            .filter(|r| !r.is_empty())
            .collect();
        let mut out = lists
            .first()
            .map(|list| (*list).clone())
            .unwrap_or_default();
        out.retain(|band| lists.iter().all(|list| list.contains(band)));
        out
    };
    let sa = bands("NRSA");
    let nsa = bands("NRNSA");
    let mut nr = sa.clone();
    nr.extend(nsa.iter().filter(|b| !sa.contains(b)).cloned());
    // 仅补充此型号的统一NR预设，不扩展未经核验的SA、NSA分项能力。
    let supplement = if key == "mt5700m-cn" {
        vec![json!(3), json!(5), json!(8)]
    } else {
        vec![]
    };
    nr.extend(supplement.iter().cloned());
    nr.sort_by_key(|v| v.as_u64());
    nr.dedup();
    let mut value = json!({"NR":nr,"NRSA":sa,"NRNSA":nsa,"LTE":bands("LTE"),"source":if rows.is_empty(){"unverified"}else{"reviewed-model-profile"},"model":key,"revision":FACTS["revision"],"transportProfiles":rows,"notHardwareVerified":true,"lockProtocolSupported":crate::lock_protocol::cell_supported(&key),"bandLockProtocolSupported":crate::lock_protocol::band_supported(&key),"scsRequired":crate::lock_protocol::scs_required(&key),"maxCells":crate::lock_protocol::max_cells(&key),"scsKHz":if crate::lock_protocol::cell_supported(&key) && crate::lock_protocol::scs_required(&key){json!([15,30,60,120,240])}else{json!([])}});
    if !supplement.is_empty() {
        value["source"] = json!("reviewed-model-supplement");
        value["supplement"] =
            json!({"NR":supplement,"scope":"unified-NR","planeSupportVerified":false});
    }
    cell_metadata(&mut value, &key);
    value
}
/// 固件协商扩大或收窄频段后也需更新，避免用旧候选范围补齐新配置。
pub fn cell_metadata(value: &mut Value, model: &str) {
    value["cellFields"] = crate::lock_protocol::cell_fields(model);
    // 与后端写入校验复用同一频点范围，只在候选唯一时由界面补齐频段。
    for (rat, ranges) in [
        ("NR", crate::radio::NR_RANGES),
        ("LTE", crate::radio::LTE_RANGES),
    ] {
        value["cellRanges"][rat] = json!(
            ranges
                .iter()
                .filter(|(band, _, _)| {
                    value[rat]
                        .as_array()
                        .is_some_and(|bands| bands.contains(&json!(band)))
                })
                .map(|(band, first, last)| json!({"band":band,"first":first,"last":last}))
                .collect::<Vec<_>>()
        );
    }
}
