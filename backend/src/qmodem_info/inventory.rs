//! 参考 QModem current_band 与 neighborcell 公开结构；读取与扫描、锁定严格分离。
use super::stamp;
use crate::{PanelError, Result, radio};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn neighbor_query_safe(row: &Value) -> bool {
    // 已审查的被动查询族；部分厂商同名方法会主动搜网或返回示例，不自动调用。
    let parser = row["profile"]["parser"].as_str().unwrap_or("");
    let manufacturer = row["manufacturer"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    if !manufacturer.is_empty() {
        return ["quectel", "fibocom", "openluat"].contains(&manufacturer.as_str());
    }
    ["quectel", "fibocom", "openluat"]
        .iter()
        .any(|prefix| parser.starts_with(prefix))
}
fn error(code: &'static str) -> PanelError {
    PanelError::new(code, 503)
}
fn text(value: &Value) -> String {
    match value {
        Value::String(s) if s.len() <= 128 && !s.chars().any(char::is_control) => {
            s.trim().to_owned()
        }
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
fn value(row: &Value, names: &[&str]) -> String {
    names
        .iter()
        .map(|key| text(&row[*key]))
        .find(|s| !s.is_empty() && s != "-")
        .unwrap_or_default()
}
fn number(raw: &str, low: f64, high: f64) -> Value {
    radio::numeric(raw, low, high).map_or(Value::Null, |v| json!(v))
}
fn rat(raw: &str) -> Option<&'static str> {
    match raw {
        "NR" | "NR5G" | "NR5G-SA" | "NR5G-NSA" | "5G" => Some("NR"),
        "LTE" | "4G" => Some("LTE"),
        _ => None,
    }
}
fn identity(row: &Value) -> String {
    format!("{}:{}:{}", row["rat"], row["channel"], row["pci"])
}
fn same_cell(a: &Value, b: &Value) -> bool {
    a["rat"] == b["rat"]
        && a["channel"] == b["channel"]
        && (a["pci"].is_null() || b["pci"].is_null() || a["pci"] == b["pci"])
}

pub fn retain_inventory(snapshot: &mut Value, previous: &Value) {
    // 同一采集代际内，仅身份完全一致时保留可选查询失败前的记录，不刷新测量时间。
    let identities = |value: &Value| {
        let mut rows: Vec<_> = value["serving"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|row| {
                json!([
                    row["rat"],
                    row["channel"],
                    row["pci"],
                    row["cellId"],
                    row["tac"],
                    row["mcc"],
                    row["mnc"]
                ])
                .to_string()
            })
            .collect();
        rows.sort();
        rows
    };
    let cells = identities(snapshot);
    let confirmed = snapshot["serving"].as_array().is_some_and(|rows| {
        rows.iter()
            .all(|row| !row["channel"].is_null() && !row["pci"].is_null())
    });
    if cells.is_empty() || !confirmed || cells != identities(previous) {
        return;
    }
    for (rows, state) in [("carriers", "carrierState"), ("neighbors", "neighborState")] {
        if !matches!(
            snapshot[state]["queryStatus"].as_str(),
            Some("error" | "pending")
        ) || !previous[state].is_object()
        {
            continue;
        }
        let old = previous[rows].as_array();
        if old.is_none_or(Vec::is_empty) {
            continue;
        }
        // 已确认的当前载波不被更旧、数量更少的库存替换。
        if rows == "carriers"
            && snapshot[rows].as_array().map_or(0, Vec::len) >= old.map_or(0, Vec::len)
        {
            continue;
        }
        let code = snapshot[state]["code"].clone();
        let query_status = snapshot[state]["queryStatus"].clone();
        snapshot[rows] = previous[rows].clone();
        snapshot[state] = previous[state].clone();
        snapshot[state]["queryStatus"] = query_status;
        snapshot[state]["code"] = code;
    }
}

pub fn attach_carriers(reply: &Value, snapshot: &mut Value, at: u64) -> Result<()> {
    let payload = &reply["current_band"];
    if payload["status"] == "unsupported" {
        return Err(error("QMODEM_CARRIERS_UNSUPPORTED"));
    }
    if payload["status"] != "ok" {
        return Err(error("QMODEM_CARRIERS_UNAVAILABLE"));
    }
    let cells = payload["cells"]
        .as_array()
        .filter(|rows| rows.len() <= 64)
        .ok_or_else(|| error("QMODEM_CARRIERS_INVALID"))?;
    let mut kept = BTreeMap::new();
    let mut anchored = false;
    for cell in cells {
        if !cell.is_object() {
            return Err(error("QMODEM_CARRIERS_INVALID"));
        }
        let Some(rat) = rat(&value(cell, &["rat"])) else {
            continue;
        };
        let role = value(cell, &["role"]);
        if !["pcc", "scc", "nsa"].contains(&role.as_str()) {
            return Err(error("QMODEM_CARRIERS_INVALID"));
        }
        let channel = value(cell, &["channel", "arfcn"]);
        let pci = value(cell, &["pci"]);
        let band = value(cell, &["band"]);
        let mut row = radio::cell(
            rat,
            &role,
            &channel,
            number(&pci, 0., if rat == "NR" { 1007. } else { 503. }),
            &band,
            "qmodem.get_current_band",
            json!({
                "channelType":"reported","dlBandwidthMHz":value(cell,&["dl_bandwidth"]),
                "ulBandwidthMHz":value(cell,&["ul_bandwidth"]),
                "scsKHz":number(&value(cell,&["scs"]),15.,240.)}),
        );
        if row["channel"].is_null() {
            return Err(error("QMODEM_CARRIERS_INVALID"));
        }
        if !pci.is_empty()
            && (row["pci"].is_null() || row["pci"].as_f64().is_some_and(|n| n.fract() != 0.))
        {
            return Err(error("QMODEM_CARRIERS_INVALID"));
        }
        if role == "pcc" || role == "nsa" {
            if !snapshot["serving"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|serving| {
                    same_cell(serving, &row)
                        && (serving["band"].is_null()
                            || row["band"].is_null()
                            || serving["band"] == row["band"])
                })
            {
                return Err(error("QMODEM_CARRIERS_CHANGED"));
            }
            anchored = true;
        }
        stamp(&mut row, at);
        let key = identity(&row);
        if kept.get(&key).is_some_and(|old| old != &row) {
            return Err(error("QMODEM_CARRIERS_AMBIGUOUS"));
        }
        kept.insert(key, row);
    }
    if !anchored || kept.is_empty() {
        return Err(error("QMODEM_CARRIERS_UNBOUND"));
    }
    let mut carriers: Vec<Value> = kept.into_values().collect();
    carriers.sort_by_key(|row| match row["role"].as_str() {
        Some("pcc") => 0,
        Some("nsa") => 1,
        _ => 2,
    });
    snapshot["carriers"] = json!(carriers);
    snapshot["carrierState"] = json!({"status":"reported","inventoryComplete":true,
        "source":"qmodem.get_current_band","observedAtMs":at,"stale":false});
    Ok(())
}

pub fn attach_neighbors(reply: &Value, snapshot: &mut Value, at: u64) -> Result<()> {
    let payload = &reply["neighborcell"];
    if !payload.is_object() || !["NR", "LTE"].iter().any(|key| payload[*key].is_array()) {
        return Err(error("QMODEM_NEIGHBORS_INVALID"));
    }
    let mut kept: BTreeMap<String, Value> = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    let mut discarded = 0;
    let mut measured = false;
    for rat in ["LTE", "NR"] {
        let cells = match payload.get(rat) {
            None => continue,
            Some(Value::Array(rows)) if rows.len() <= 512 => rows,
            _ => return Err(error("QMODEM_NEIGHBORS_INVALID")),
        };
        for cell in cells {
            if !cell.is_object() {
                discarded += 1;
                continue;
            }
            let kind = value(cell, &["neighbourcell"]).to_ascii_lowercase();
            if kind.starts_with("ex") || kind.contains("example") {
                discarded += 1;
                continue;
            }
            let channel = value(cell, &["arfcn", "channel"]);
            let pci = value(cell, &["pci"]);
            let mut rsrp = number(&value(cell, &["rsrp"]), -156., -28.);
            let mut rsrq = number(&value(cell, &["rsrq"]), -44., 20.);
            let mut encoding = "reported";
            // 旧版移远脚本把 LTE intra/inter 两列颠倒；仅在原解释无效且反向唯一合法时纠正。
            if rat == "LTE"
                && (kind.contains("intra") || kind.contains("inter"))
                && (rsrp.is_null() || rsrq.is_null())
            {
                let reversed_rsrp = number(&value(cell, &["rsrq"]), -156., -28.);
                let reversed_rsrq = number(&value(cell, &["rsrp"]), -44., 20.);
                if !reversed_rsrp.is_null() && !reversed_rsrq.is_null() {
                    rsrp = reversed_rsrp;
                    rsrq = reversed_rsrq;
                    encoding = "legacy-lte-columns-corrected";
                }
            }
            let mut row = radio::cell(
                rat,
                "neighbor",
                &channel,
                number(&pci, 0., if rat == "NR" { 1007. } else { 503. }),
                &value(cell, &["band"]),
                "qmodem.get_neighborcell",
                json!({
                    "rsrp":rsrp,"rsrq":rsrq,"sinr":value(cell,&["sinr"]),
                    "rssi":value(cell,&["rssi"]),"metricEncoding":encoding,
                    "encodedMetricsUnresolved":cell.get("ss_rsrp").is_some() || cell.get("ss_sinr").is_some()}),
            );
            if row["channel"].is_null() || row["pci"].is_null() {
                discarded += 1;
                continue;
            }
            measured = true;
            if snapshot["serving"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(snapshot["carriers"].as_array().into_iter().flatten())
                .any(|serving| same_cell(serving, &row))
            {
                discarded += 1;
                continue;
            }
            stamp(&mut row, at);
            let key = identity(&row);
            if conflicts.contains(&key) {
                discarded += 1;
                continue;
            }
            if kept.get(&key).is_some_and(|old| old != &row) {
                kept.remove(&key);
                conflicts.insert(key);
                discarded += 1;
                continue;
            }
            kept.insert(key, row);
        }
    }
    let count = kept.len();
    if count == 0 && (!conflicts.is_empty() || discarded > 0 && !measured) {
        return Err(error("QMODEM_NEIGHBORS_INVALID"));
    }
    snapshot["neighbors"] = json!(kept.into_values().collect::<Vec<_>>());
    snapshot["neighborState"] = json!({"status":if count==0 {"empty"}else{"reported"},
        "reason":if count==0 {Some("NO_NEIGHBOR_MEASUREMENTS")}else{None},"discarded":discarded,
        "source":"qmodem.get_neighborcell","observedAtMs":at,"stale":false});
    Ok(())
}
