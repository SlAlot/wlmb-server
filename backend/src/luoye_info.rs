//! 落叶小通魔改 modem 的已发布快照转换；字段与事务契约参考作者提供的接入说明。
use crate::{PanelError, Result, radio};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const PROVIDER: &str = "luoye-modem-cache";
pub const MAX_CACHE: usize = 262144;
fn invalid() -> PanelError {
    PanelError::new("MODEM_CACHE_INVALID", 503)
}
pub fn text(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| s.len() <= 256 && !s.chars().any(char::is_control))
}
fn known(value: &str) -> &str {
    if ["-", "unknown", "Unknown", "null", "N/A"].contains(&value) {
        ""
    } else {
        value
    }
}
pub fn payload(raw: &str) -> Result<Value> {
    if raw.len() > MAX_CACHE {
        return Err(invalid());
    }
    let mut value: Value = serde_json::from_str(raw).map_err(|_| invalid())?;
    if value["modem_info"]["more_info"].is_object() {
        value = value["modem_info"]["more_info"].clone();
    }
    if !value["base_info"].is_object()
        || !value["cell_info"].is_object()
        || !value["network_info"].is_array()
        || !value["sim_info"].is_array()
    {
        return Err(invalid());
    }
    Ok(value)
}
fn fields(value: &Value) -> Result<BTreeMap<String, String>> {
    let rows = value
        .as_array()
        .filter(|v| v.len() <= 256)
        .ok_or_else(invalid)?;
    let mut result = BTreeMap::new();
    for row in rows {
        let row = row
            .as_object()
            .filter(|v| v.len() <= 8)
            .ok_or_else(invalid)?;
        for (key, value) in row {
            if key == "full_name" {
                continue;
            }
            if key.len() > 80 {
                return Err(invalid());
            }
            let value = known(text(value).ok_or_else(invalid)?);
            if result.get(key).is_some_and(|old| old != value) {
                return Err(invalid());
            }
            result.insert(key.clone(), value.to_owned());
        }
    }
    Ok(result)
}
fn get<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    fields.get(key).map(String::as_str).unwrap_or("")
}
fn quantity(raw: &str, unit: &str, low: f64, high: f64) -> Value {
    let raw = raw.trim();
    let raw = raw.strip_suffix(unit).unwrap_or(raw).trim();
    radio::numeric(raw, low, high).map_or(Value::Null, |v| json!(v))
}
fn integer(raw: &str, high: f64) -> Value {
    radio::numeric(raw, 0., high)
        .filter(|v| v.fract() == 0.)
        .map_or(Value::Null, |v| json!(v))
}
fn stamp(row: &mut Value, at: Option<u64>, stale: bool) {
    row["observedAtMs"] = json!(at);
    row["sourcePublishedAtMs"] = json!(at);
    row["observationTimeBasis"] = json!("source-file-publication");
    row["sourceObservationTimeUnknown"] = json!(true);
    row["providerCached"] = json!(true);
    row["sourceCacheAgeUnknown"] = json!(at.is_none());
    row["stale"] = json!(stale);
}
fn serving(rat: &str, role: &str, mode: &str, fields: &BTreeMap<String, String>) -> Value {
    let mut row = radio::cell(
        rat,
        role,
        get(fields, if rat == "NR" { "ARFCN" } else { "EARFCN" }),
        integer(
            get(fields, "Physical Cell ID"),
            if rat == "NR" { 1007. } else { 503. },
        ),
        get(fields, "Band"),
        PROVIDER,
        json!({
            "networkMode":mode,"channelType":"unknown",
            "rsrp":quantity(get(fields,"RSRP"),"dBm",-156.,-28.),
            "rsrq":quantity(get(fields,"RSRQ"),"dB",-44.,20.),
            "sinr":quantity(get(fields,"SINR"),"dB",-23.,50.),
            "rssi":quantity(get(fields,"RSSI"),"dBm",-140.,0.),
            "dlBandwidthMHz":quantity(get(fields,"DL Bandwidth"),"MHz",0.1,1000.),
            "ulBandwidthMHz":quantity(get(fields,"UL Bandwidth"),"MHz",0.1,1000.)}),
    );
    for key in ["MCC", "MNC", "TAC", "Cell ID"] {
        let value = get(fields, key);
        let valid = !value.is_empty()
            && if key == "MCC" || key == "MNC" {
                (if key == "MCC" { 3..=3 } else { 2..=3 }).contains(&value.len())
                    && value.bytes().all(|b| b.is_ascii_digit())
            } else {
                value.len() <= 32
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() || b"xX".contains(&b))
            };
        row[match key {
            "MCC" => "mcc",
            "MNC" => "mnc",
            "TAC" => "tac",
            _ => "cellId",
        }] = if valid { json!(value) } else { Value::Null };
    }
    let scs = quantity(get(fields, "SCS"), "kHz", 15., 240.);
    row["scsKHz"] = if rat == "NR"
        && [15., 30., 60., 120., 240.]
            .iter()
            .any(|v| scs.as_f64() == Some(*v))
    {
        scs
    } else {
        Value::Null
    };
    let mut csi = json!({});
    for (key, target, max) in [
        ("DL MCS", "dlMcs", 31.),
        ("UL MCS", "ulMcs", 31.),
        ("RI", "ri", 8.),
        ("CSI CQI", "cqi", 15.),
        ("PMI", "pmi", 65535.),
    ] {
        csi[target] = integer(get(fields, key), max);
    }
    for (key, target) in [
        ("DL Modulation", "dlModulation"),
        ("UL Modulation", "ulModulation"),
        ("CSI Status", "status"),
    ] {
        let value = get(fields, key);
        csi[target] = if !value.is_empty() && value.len() <= 32 && !value.contains(['<', '>']) {
            json!(value)
        } else {
            Value::Null
        };
    }
    row["csi"] = csi;
    row
}
fn carriers(fields: &BTreeMap<String, String>) -> Result<Vec<Value>> {
    let keys = [
        "Carrier Aggregation",
        "CA Band",
        "CA DL ARFCN",
        "CA DL Bandwidth",
        "CA UL ARFCN",
        "CA UL Bandwidth",
        "CA DL Frequency",
        "CA UL Frequency",
    ];
    if keys.iter().all(|key| get(fields, key).is_empty()) {
        return Ok(vec![]);
    }
    let cols: Vec<Vec<_>> = keys
        .iter()
        .map(|key| get(fields, key).split('|').map(str::trim).collect())
        .collect();
    let count = cols[0].len();
    if count > 32
        || cols[..3].iter().any(|v| v.len() != count || v == &[""])
        || cols[3..].iter().any(|v| v != &[""] && v.len() != count)
    {
        return Err(invalid());
    }
    let mut result = vec![];
    for i in 0..count {
        let col = |n: usize| known(cols[n].get(i).copied().unwrap_or(""));
        let band = col(1);
        let rat = if band.starts_with(['N', 'n']) {
            "NR"
        } else if band.starts_with(['B', 'b']) {
            "LTE"
        } else {
            return Err(invalid());
        };
        let role = match col(0) {
            "PCC" => "pcc",
            "SCC" => "scc",
            _ => "unknown",
        };
        let row = radio::cell(
            rat,
            role,
            col(2),
            Value::Null,
            band,
            PROVIDER,
            json!({
            "channelType":"center","dlBandwidthMHz":quantity(col(3),"MHz",0.1,1000.),
            "ulBandwidthMHz":quantity(col(5),"MHz",0.1,1000.),
            "ulChannel":integer(col(4),if rat=="NR"{3279165.}else{262143.}),
            "dlFrequencyMHz":quantity(col(6),"MHz",1.,100000.),"ulFrequencyMHz":quantity(col(7),"MHz",1.,100000.)}),
        );
        if row["channel"].is_null() || row["band"].is_null() {
            return Err(invalid());
        }
        result.push(row);
    }
    Ok(result)
}
pub fn identity(value: &Value) -> Option<Vec<String>> {
    let rows = value["serving"].as_array().filter(|v| !v.is_empty())?;
    let mut keys = vec![];
    for row in rows {
        if row["channel"].is_null() || row["pci"].is_null() {
            return None;
        }
        keys.push(
            json!([
                row["rat"],
                row["channel"],
                row["pci"],
                row["cellId"],
                row["tac"],
                row["mcc"],
                row["mnc"]
            ])
            .to_string(),
        );
    }
    keys.sort();
    Some(keys)
}
pub fn parse(value: &Value, published: Option<u64>, trusted: bool, now: u64) -> Result<Value> {
    let stale = !trusted || published.is_none_or(|at| at > now || now - at > 15000);
    let mut result = json!({"serving":[],"carriers":[],"neighbors":[],"observations":[],"errors":[],
        "pending":false,"partial":false,"receivedAtMs":now,"source":PROVIDER});
    let cell = value["cell_info"].as_object().ok_or_else(invalid)?;
    let mut groups = vec![];
    for (mode, rat, role, label) in [
        ("NR5G-SA Mode", "NR", "pcc", "5G SA"),
        ("LTE Mode", "LTE", "pcc", "4G LTE"),
    ] {
        if let Some(rows) = cell.get(mode) {
            groups.push((rat, role, label, fields(rows)?));
        }
    }
    if let Some(rows) = cell.get("EN-DC Mode") {
        for group in rows
            .as_array()
            .filter(|v| v.len() <= 8)
            .ok_or_else(invalid)?
        {
            let group = group.as_object().ok_or_else(invalid)?;
            for (key, rat, role) in [("LTE", "LTE", "pcc"), ("NR5G-NSA", "NR", "nsa")] {
                if let Some(rows) = group.get(key) {
                    groups.push((rat, role, "5G NSA", fields(rows)?));
                }
            }
        }
    }
    if groups.len() > 2
        || groups.len() == 2
            && (groups.iter().any(|(_, _, mode, _)| *mode != "5G NSA")
                || groups[0].0 == groups[1].0)
    {
        return Err(invalid());
    }
    groups.sort_by_key(|(_, role, _, _)| if *role == "pcc" { 0 } else { 1 });
    let mut serving_rows = vec![];
    let mut ca = vec![];
    let mut saw_ca = false;
    for (rat, role, mode, fields) in &groups {
        let row = serving(rat, role, mode, fields);
        if !row["channel"].is_null() || !row["pci"].is_null() {
            serving_rows.push(row);
        }
        match carriers(fields) {
            Ok(rows) if !rows.is_empty() => {
                if saw_ca && ca != rows {
                    return Err(invalid());
                }
                ca = rows;
                saw_ca = true;
            }
            Ok(_) => {}
            Err(_) => {
                result["errors"]
                    .as_array_mut()
                    .ok_or_else(invalid)?
                    .push(json!({"code":"MODEM_CACHE_CA_INVALID"}));
            }
        }
    }
    if ca.is_empty() {
        ca = serving_rows.clone();
    }
    for rows in [&mut serving_rows, &mut ca] {
        for row in rows {
            stamp(row, published, stale);
        }
    }
    result["serving"] = json!(serving_rows);
    result["carriers"] = json!(ca);
    result["carrierState"] = json!({"status":if saw_ca {"reported"}else{"serving-only"},"inventoryComplete":false,"source":PROVIDER,"observedAtMs":published,"stale":stale});
    result["neighborState"] = json!({"status":"unsupported","reason":"NEIGHBOR_CHANNEL_NOT_READY"});
    let base = &value["base_info"];
    let temperature = quantity(text(&base["temperature"]).unwrap_or(""), "°C", -40., 150.);
    if !temperature.is_null() {
        result["temperature"] = json!({"celsius":temperature,"source":PROVIDER});
        stamp(&mut result["temperature"], published, stale);
    }
    let network = fields(&value["network_info"])?;
    let rate = |raw: &str| {
        let raw = raw.trim();
        for (unit, factor) in [
            ("Gbps", 1e6),
            ("Mbps", 1000.),
            ("Kbps", 1.),
            ("kbps", 1.),
            ("bps", 0.001),
        ] {
            if let Some(number) = raw.strip_suffix(unit) {
                return radio::numeric(number, 0., 1e10)
                    .map(|v| v * factor)
                    .filter(|v| *v <= 1e10);
            }
        }
        None
    };
    result["subscription"] = json!({"kind":"AMBR","source":PROVIDER,"dlKbps":rate(get(&network,"AMBR DL")),"ulKbps":rate(get(&network,"AMBR UL")),
        "cqiDl":integer(get(&network,"CQI DL"),15.),"cqiUl":integer(get(&network,"CQI UL"),15.)});
    stamp(&mut result["subscription"], published, stale);
    crate::network_operator::attach("", &mut result);
    if result["networkOperator"].is_object() {
        stamp(&mut result["networkOperator"], published, stale);
    }
    let sim = fields(&value["sim_info"])?;
    result["simStatus"] = json!(get(&sim, "SIM Status"));
    result["connected"] = if stale {
        Value::Null
    } else {
        match text(&base["connect_status"]) {
            Some("connect") => json!(true),
            Some("disconnect") => json!(false),
            _ => Value::Null,
        }
    };
    stamp(&mut result, published, stale);
    result["partial"] = json!(
        result["serving"].as_array().is_none_or(Vec::is_empty)
            || !result["errors"].as_array().is_some_and(Vec::is_empty)
    );
    if cell.contains_key("WCDMA Mode") {
        result["errors"]
            .as_array_mut()
            .ok_or_else(invalid)?
            .push(json!({"code":"MODEM_CACHE_RAT_UNSUPPORTED"}));
        result["partial"] = json!(true);
    }
    Ok(result)
}
