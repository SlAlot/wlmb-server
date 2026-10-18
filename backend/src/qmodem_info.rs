//! 参考 QModem rpcd 的公开 modem_info 字段，自有解析实现；只接收信息，不推断写入能力。
use crate::{PanelError, Result, radio};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn supported(schema: &Value) -> bool {
    schema["cell_info"]["config_section"] == 3
}

pub fn method_supported(schema: &Value, method: &str) -> bool {
    matches!(method, "cell_info" | "network_info" | "base_info")
        && schema[method]["config_section"] == 3
}

fn entries(value: &Value) -> Result<&[Value]> {
    let rows = value["modem_info"]
        .as_array()
        .filter(|rows| rows.len() <= 512)
        .ok_or_else(|| PanelError::new("QMODEM_INFO_INVALID", 503))?;
    if rows.iter().any(|row| {
        !row.is_object()
            || row["key"].as_str().is_none_or(|s| s.len() > 128)
            || row["value"]
                .as_str()
                .is_none_or(|s| s.len() > 4096 || s.chars().any(char::is_control))
    }) {
        return Err(PanelError::new("QMODEM_INFO_INVALID", 503));
    }
    Ok(rows)
}

type Fields = BTreeMap<String, String>;
fn put(fields: &mut Fields, key: &str, value: &str) -> Result<()> {
    if fields.get(key).is_some_and(|old| old != value) {
        return Err(PanelError::new("QMODEM_INFO_AMBIGUOUS", 503));
    }
    fields.insert(key.to_owned(), value.to_owned());
    Ok(())
}
fn field<'a>(fields: &'a Fields, names: &[&str]) -> &'a str {
    names
        .iter()
        .find_map(|key| fields.get(*key))
        .map(String::as_str)
        .unwrap_or("")
}
fn num(value: &str, low: f64, high: f64) -> Value {
    radio::numeric(value, low, high).map_or(Value::Null, |v| json!(v))
}
fn stamp(row: &mut Value, at: u64) {
    row["observedAtMs"] = json!(at);
    row["stale"] = json!(false);
    row["providerCached"] = json!(true);
    row["sourceCacheAgeUnknown"] = json!(true);
}

pub fn parse(cell: &Value, network: Option<&Value>, at: u64) -> Result<Value> {
    let rows = entries(cell)?;
    let modes: Vec<_> = rows
        .iter()
        .filter(|row| row["key"] == "network_mode")
        .filter_map(|row| row["value"].as_str())
        .collect();
    if modes.len() != 1 {
        return Err(PanelError::new("QMODEM_INFO_INVALID", 503));
    }
    let mode = modes[0].to_ascii_uppercase();
    let nsa = mode.contains("EN-DC") || mode.contains("NSA");
    let default = if nsa || mode.contains("LTE") {
        "LTE"
    } else if mode.contains("NR5G") || mode.contains("5G SA") {
        "NR"
    } else {
        return Err(PanelError::new("QMODEM_INFO_UNSUPPORTED", 422));
    };
    let mut current = default;
    let mut groups: BTreeMap<String, Fields> = BTreeMap::new();
    for row in rows {
        let key = row["key"].as_str().unwrap_or("");
        let value = row["value"].as_str().unwrap_or("");
        match key {
            "LTE" => {
                current = "LTE";
                continue;
            }
            "NR5G-NSA" | "NR5G-SA" | "NR" => {
                current = "NR";
                continue;
            }
            _ => {}
        }
        let group = match row["extra_info"].as_str() {
            Some("LTE" | "NR" | "NR5G-SA" | "NR5G-NSA" | "CA-NR" | "CA-LTE") => {
                match row["extra_info"].as_str().unwrap_or("") {
                    "NR5G-SA" | "NR5G-NSA" => "NR",
                    label => label,
                }
            }
            _ if key.ends_with(" (CA)") => {
                if current == "NR" {
                    "CA-NR"
                } else {
                    "CA-LTE"
                }
            }
            _ => current,
        };
        // 不保留未使用字段，尤其不把身份号码或任意扩展内容透传到快照。
        if [
            "MCC",
            "MNC",
            "TAC",
            "Cell ID",
            "Physical Cell ID",
            "PCI",
            "ARFCN",
            "EARFCN",
            "Band",
            "UL Bandwidth",
            "DL Bandwidth",
            "RSRP",
            "RSRQ",
            "SINR",
            "RSSI",
            "SCS",
            "Physical Cell ID (CA)",
            "ARFCN (CA)",
            "EARFCN (CA)",
            "Band (CA)",
            "UL Bandwidth (CA)",
            "DL Bandwidth (CA)",
        ]
        .contains(&key)
        {
            put(groups.entry(group.to_owned()).or_default(), key, value)?;
        }
    }
    let source = "qmodem.cell_info";
    let mut serving = vec![];
    for rat in ["LTE", "NR"] {
        let Some(fields) = groups.get(rat) else {
            continue;
        };
        let channel = field(fields, if rat == "NR" { &["ARFCN"] } else { &["EARFCN"] });
        if radio::numeric(channel, 0., if rat == "NR" { 3279165. } else { 262143. })
            .is_none_or(|v| v.fract() != 0.)
        {
            continue;
        }
        let mut row = radio::cell(
            rat,
            if rat == "NR" && nsa { "nsa" } else { "pcc" },
            channel,
            num(
                field(fields, &["Physical Cell ID", "PCI"]),
                0.,
                if rat == "NR" { 1007. } else { 503. },
            ),
            field(fields, &["Band"]),
            source,
            json!({
                "networkMode":if nsa { "5G NSA" } else if rat == "NR" { "5G SA" } else { "4G LTE" },
                "mcc":field(fields, &["MCC"]),"mnc":field(fields, &["MNC"]),
                "tac":field(fields, &["TAC"]),"cellId":field(fields, &["Cell ID"]),
                "rsrp":field(fields, &["RSRP"]),"rsrq":field(fields, &["RSRQ"]),
                "sinr":field(fields, &["SINR"]),"rssi":field(fields, &["RSSI"]),
                "dlBandwidthMHz":field(fields, &["DL Bandwidth"]),
                "ulBandwidthMHz":field(fields, &["UL Bandwidth"]),
                "scsKHz":num(field(fields, &["SCS"]),15.,240.)}),
        );
        stamp(&mut row, at);
        serving.push(row);
    }
    if serving.is_empty() {
        return Err(PanelError::new("QMODEM_INFO_EMPTY", 503));
    }
    let mut carriers = vec![];
    for (group, fields) in &groups {
        let rat = match group.as_str() {
            "CA-NR" => "NR",
            "CA-LTE" => "LTE",
            _ => continue,
        };
        let channels: Vec<_> = field(fields, &["ARFCN (CA)", "EARFCN (CA)"])
            .split('/')
            .map(str::trim)
            .collect();
        if channels.len() > 16 {
            continue;
        }
        let keys = [
            "Physical Cell ID (CA)",
            "Band (CA)",
            "DL Bandwidth (CA)",
            "UL Bandwidth (CA)",
        ];
        let columns: Vec<Vec<_>> = keys
            .iter()
            .map(|key| field(fields, &[*key]).split('/').map(str::trim).collect())
            .collect();
        if columns
            .iter()
            .any(|values| values != &[""] && values.len() != channels.len())
        {
            continue;
        }
        for (i, channel) in channels.iter().enumerate() {
            let column = |n: usize| columns[n].get(i).copied().unwrap_or("");
            let mut row = radio::cell(
                rat,
                "scc",
                channel,
                num(column(0), 0., if rat == "NR" { 1007. } else { 503. }),
                column(1),
                source,
                json!({"dlBandwidthMHz":column(2),"ulBandwidthMHz":column(3)}),
            );
            if row["channel"].is_null() {
                continue;
            }
            stamp(&mut row, at);
            carriers.push(row);
        }
    }
    if !carriers.is_empty() {
        carriers.splice(0..0, serving.iter().cloned());
    }
    let mut result = json!({"serving":serving,"carriers":carriers,"neighbors":[],"errors":[],
        "observations":[{"command":source,"observedAtMs":at}],"observedAtMs":at,
        "partial":false,"pending":false,"stale":false,"providerCached":true,"sourceCacheAgeUnknown":true});
    crate::network_operator::attach("", &mut result);
    if let Some(operator) = result.get_mut("networkOperator") {
        stamp(operator, at);
    }
    if let Some(network) = network {
        attach_network(network, &mut result, at)?;
    }
    Ok(result)
}

pub fn attach_network(network: &Value, result: &mut Value, at: u64) -> Result<()> {
    let mut fields = Fields::new();
    for row in entries(network)? {
        let key = row["key"].as_str().unwrap_or("");
        if ["AMBR DL", "AMBR UL", "CQI UL", "CQI DL"].contains(&key) {
            put(&mut fields, key, row["value"].as_str().unwrap_or(""))?;
        }
    }
    if let (Some(dl), Some(ul)) = (
        ambr_kbps(field(&fields, &["AMBR DL"])),
        ambr_kbps(field(&fields, &["AMBR UL"])),
    ) {
        let mut value = json!({"dlKbps":dl,"ulKbps":ul,"kind":"AMBR",
            "source":"qmodem.network_info","cqiDl":num(field(&fields,&["CQI DL"]),0.,15.),
            "cqiUl":num(field(&fields,&["CQI UL"]),0.,15.)});
        stamp(&mut value, at);
        result["subscription"] = value;
    }
    Ok(())
}

fn ambr_kbps(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    let end = raw
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(raw.len());
    let multiplier = match raw[end..].trim() {
        // 移远与美格的无单位字段为 Mbps；有单位的公共回复严格区分 bit 与 byte。
        "" | "Mbps" => 1000.,
        "Kbps" | "kbps" => 1.,
        "Gbps" => 1e6,
        "bps" => 0.001,
        "MB/s" => 8000.,
        "KB/s" => 8.,
        _ => return None,
    };
    let value = radio::numeric(&raw[..end], 0.001, 1e10)? * multiplier;
    (value <= 1e10).then_some(value)
}

pub fn attach_base(base: &Value, result: &mut Value, at: u64) -> Result<()> {
    let values: Vec<_> = entries(base)?
        .iter()
        .filter(|row| row["key"] == "temperature")
        .collect();
    if values.len() == 1 {
        let raw = values[0]["value"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_end_matches(['C', 'c', '℃', '°'])
            .trim();
        if let Some(celsius) = radio::numeric(raw, -40., 150.) {
            let mut sensor = json!({"celsius":celsius,"source":"qmodem.base_info"});
            stamp(&mut sensor, at);
            result["temperature"] = sensor;
        }
    }
    Ok(())
}
