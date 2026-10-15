//! 参考QModem各平台公开锁定协议；确认、保存回读和射频生效分别处理。
use crate::{PanelError, Result, catalog, controls, radio};
use serde_json::{Value, json};
pub fn unisoc(model: &str) -> bool {
    ["rg200u-cn", "rm500u-cn", "rm500u-cnv", "rm500u-ea"]
        .contains(&catalog::normalize(model).as_str())
}
pub fn cell_supported(model: &str) -> bool {
    catalog::normalize(model) == "mt5700m-cn"
        || crate::vendor_cell_lock::supported(model)
        || unisoc(model)
        || crate::quectel_lock::platform(model).is_some()
}
pub fn immediate(model: &str) -> bool {
    unisoc(model)
        || crate::quectel_lock::platform(model).is_some()
        || crate::vendor_cell_lock::supported(model)
}
pub fn scs_required(model: &str) -> bool {
    !unisoc(model) && crate::quectel_lock::platform(model) != Some("lte")
}
/// 前端只请求对应协议实际使用的参数，不把其他平台的输入项套给所有模组。
pub fn cell_fields(model: &str) -> Value {
    json!({
        "NR": {"bandRequired": !unisoc(model), "scsRequired": scs_required(model)},
        "LTE": {"bandRequired": catalog::normalize(model) == "mt5700m-cn", "scsRequired": false}
    })
}
pub fn max_cells(model: &str) -> usize {
    if unisoc(model) {
        6
    } else if crate::quectel_lock::platform(model).is_some()
        || crate::vendor_cell_lock::supported(model)
    {
        1
    } else {
        4
    }
}
pub fn band_supported(model: &str) -> bool {
    catalog::normalize(model) == "mt5700m-cn" || crate::band_locks::supported(model)
}
pub fn query(model: &str, rat: &str) -> String {
    if crate::vendor_cell_lock::supported(model) {
        crate::vendor_cell_lock::query(model, rat).unwrap_or_default()
    } else if crate::quectel_lock::platform(model).is_some() {
        crate::quectel_lock::query(model, rat).unwrap_or_default()
    } else if unisoc(model) {
        format!(
            "AT+QNWLOCK=\"common/{}\"",
            if rat == "NR" { "5g" } else { "lte" }
        )
    } else {
        format!("AT^{}FREQLOCK?", if rat == "NR" { "NR" } else { "LTE" })
    }
}
pub fn command(model: &str, operation: &Value) -> Result<String> {
    if crate::vendor_cell_lock::supported(model) {
        return crate::vendor_cell_lock::command(model, operation);
    }
    if catalog::normalize(model) == "mt5700m-cn" && operation["target"] == "band" {
        let mut operation = operation.clone();
        operation
            .as_object_mut()
            .ok_or_else(|| PanelError::new("LOCK_INVALID", 400))?
            .remove("target");
        return controls::lock_command(&operation);
    }
    if !cell_supported(model) {
        return Err(PanelError::new("LOCK_UNSUPPORTED", 422));
    }
    if crate::quectel_lock::platform(model).is_some() {
        return crate::quectel_lock::command(model, operation);
    }
    if !unisoc(model) {
        return controls::lock_command(operation);
    }
    if operation["mode"] == "band" {
        return Err(PanelError::new("LOCK_BAND_PROTOCOL_UNSUPPORTED", 422));
    }
    let invalid = || PanelError::new("LOCK_INVALID", 400);
    let rat = operation["rat"]
        .as_str()
        .filter(|v| ["NR", "LTE"].contains(v))
        .ok_or_else(invalid)?;
    if !operation.as_object().is_some_and(|m| {
        m.len() == 3
            && ["rat", "mode", "entries"]
                .iter()
                .all(|k| m.contains_key(*k))
    }) || !["auto", "cell"].contains(&operation["mode"].as_str().unwrap_or(""))
    {
        return Err(invalid());
    }
    let prefix = query(model, rat);
    let entries = operation["entries"]
        .as_array()
        .ok_or_else(|| PanelError::new("LOCK_INVALID", 400))?;
    if entries.len() > 6 || (operation["mode"] == "auto") != entries.is_empty() {
        return Err(invalid());
    }
    for (index, entry) in entries.iter().enumerate() {
        if !entry.as_object().is_some_and(|m| {
            m.contains_key("arfcn")
                && m.contains_key("pci")
                && m.keys().all(|k| {
                    ["band", "arfcn", "pci", "scsType", "bandCandidates"].contains(&k.as_str())
                })
        }) || entries[..index]
            .iter()
            .any(|e| e["arfcn"] == entry["arfcn"] && e["pci"] == entry["pci"])
        {
            return Err(invalid());
        }
        if entry["arfcn"]
            .as_u64()
            .is_none_or(|v| v > if rat == "NR" { 3279165 } else { 262143 })
            || entry["pci"]
                .as_u64()
                .is_none_or(|v| v > if rat == "NR" { 1007 } else { 503 })
        {
            return Err(invalid());
        }
        if !entry["scsType"].is_null() && entry["scsType"].as_u64().is_none_or(|v| v > 4) {
            return Err(invalid());
        }
    }
    let fields = entries
        .iter()
        .map(|e| format!(",{},{}", e["arfcn"], e["pci"]))
        .collect::<String>();
    Ok(format!("{prefix},{}{fields}", entries.len()))
}
pub fn parse(model: &str, raw: &str, rat: &str) -> Result<Value> {
    if crate::vendor_cell_lock::supported(model) {
        return crate::vendor_cell_lock::parse(model, raw, rat);
    }
    if !cell_supported(model) {
        return Err(PanelError::new("LOCK_UNSUPPORTED", 422));
    }
    if crate::quectel_lock::platform(model).is_some() {
        return crate::quectel_lock::parse(model, raw, rat);
    }
    if !unisoc(model) {
        return controls::parse_lock(raw, rat);
    }
    let invalid = || PanelError::new("LOCK_UNVERIFIED", 422);
    if raw.len() > crate::MAX_AT_RESPONSE || raw.contains('\0') || !["NR", "LTE"].contains(&rat) {
        return Err(invalid());
    }
    let mut lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect();
    let echo = query(model, rat);
    if lines.first() == Some(&echo.as_str()) {
        lines.remove(0);
    }
    if lines.pop() != Some("OK") {
        return Err(invalid());
    }
    let standard = if rat == "NR" {
        "\"common/5g\""
    } else {
        "\"common/lte\""
    };
    let mut entries = vec![];
    for line in lines {
        let parts: Vec<_> = line
            .strip_prefix("+QNWLOCK:")
            .ok_or_else(invalid)?
            .split(',')
            .map(str::trim)
            .collect();
        if parts.len() != 3 || parts[0] != standard || entries.len() >= 6 {
            return Err(invalid());
        }
        let integer = |s: &str, max: u64| -> Result<u64> {
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            s.parse::<u64>()
                .ok()
                .filter(|v| *v <= max)
                .ok_or_else(invalid)
        };
        let channel = integer(parts[1], if rat == "NR" { 3279165 } else { 262143 })?;
        let pci = integer(parts[2], if rat == "NR" { 1007 } else { 503 })?;
        let candidates = radio::band_candidates(rat, channel as f64);
        let band = candidates
            .first()
            .filter(|_| candidates.len() == 1)
            .and_then(|b| b[1..].parse::<u64>().ok());
        let row = json!({"arfcn":channel,"pci":pci,"band":band,"bandCandidates":candidates,"scsType":null});
        if entries.contains(&row) {
            return Err(invalid());
        }
        entries.push(row);
    }
    Ok(
        json!({"rat":rat,"mode":if entries.is_empty(){0}else{2},"entries":entries,"scsRequired":false,"persistent":false,"reportedFields":["arfcn","pci"]}),
    )
}
pub fn matched(model: &str, group: &Value, operation: &Value) -> bool {
    if !immediate(model) {
        return false;
    }
    let wanted = operation["entries"].as_array();
    let actual = group["entries"].as_array();
    match (actual, wanted) {
        (Some(actual), Some(wanted)) => {
            actual.len() == wanted.len()
                && wanted.iter().all(|w| {
                    actual.iter().any(|a| {
                        a["arfcn"] == w["arfcn"]
                            && a["pci"] == w["pci"]
                            && (group["rat"] != "NR"
                                || unisoc(model)
                                || a["band"] == w["band"] && a["scsType"] == w["scsType"])
                    })
                })
                && group["mode"] == if wanted.is_empty() { 0 } else { 2 }
        }
        _ => false,
    }
}
