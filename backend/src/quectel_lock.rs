//! 移远平台协议格式参考QModem公开适配；自有解析与校验，不跨平台猜测指令。
use crate::{PanelError, Result, catalog, radio};
use serde_json::{Value, json};

pub fn platform(model: &str) -> Option<&'static str> {
    let profile = catalog::identify(model);
    if profile["family"] != "quectel" {
        return None;
    }
    match catalog::platform(profile["model"].as_str()?) {
        "qualcomm" | "lte12" => Some("qualcomm"),
        "lte" => Some("lte"),
        _ => None,
    }
}
pub fn query(model: &str, rat: &str) -> Result<String> {
    let suffix = match (platform(model), rat) {
        (Some("qualcomm"), "NR") => "5g",
        (Some("qualcomm"), "LTE") => "4g",
        (Some("lte"), "LTE") => "lte",
        _ => return Err(PanelError::new("LOCK_UNSUPPORTED", 422)),
    };
    Ok(format!("AT+QNWLOCK=\"common/{suffix}\""))
}
fn integer(v: &Value, max: u64) -> Result<u64> {
    v.as_u64()
        .filter(|n| *n <= max)
        .ok_or_else(|| PanelError::new("LOCK_INVALID", 400))
}
pub fn command(model: &str, op: &Value) -> Result<String> {
    let invalid = || PanelError::new("LOCK_INVALID", 400);
    if !op.as_object().is_some_and(|m| {
        m.len() == 3
            && ["rat", "mode", "entries"]
                .iter()
                .all(|k| m.contains_key(*k))
    }) {
        return Err(invalid());
    }
    let rat = op["rat"].as_str().ok_or_else(invalid)?;
    let prefix = query(model, rat)?;
    let rows = op["entries"].as_array().ok_or_else(invalid)?;
    if op["mode"] == "band" {
        return Err(PanelError::new("LOCK_BAND_PROTOCOL_UNSUPPORTED", 422));
    }
    if op["mode"] == "auto" && rows.is_empty() {
        return Ok(format!("{prefix},0"));
    }
    if op["mode"] != "cell" || rows.len() != 1 {
        return Err(invalid());
    }
    let row = &rows[0];
    if !row.as_object().is_some_and(|m| {
        m.keys()
            .all(|k| ["band", "arfcn", "pci", "scsType", "bandCandidates"].contains(&k.as_str()))
    }) {
        return Err(invalid());
    }
    let channel = integer(&row["arfcn"], if rat == "NR" { 3279165 } else { 262143 })?;
    let pci = integer(&row["pci"], if rat == "NR" { 1007 } else { 503 })?;
    if rat == "NR" {
        let band = integer(&row["band"], 1024)?;
        let scs = integer(&row["scsType"], 4)?;
        if !radio::band_candidates(rat, channel as f64).contains(&format!("n{band}")) {
            return Err(invalid());
        }
        Ok(format!("{prefix},{pci},{channel},{},{band}", 15u64 << scs))
    } else {
        Ok(format!(
            "{prefix},{},{channel},{pci}",
            if platform(model) == Some("lte") { 2 } else { 1 }
        ))
    }
}
pub fn parse(model: &str, raw: &str, rat: &str) -> Result<Value> {
    let invalid = || PanelError::new("LOCK_UNVERIFIED", 422);
    let echo = query(model, rat)?;
    if raw.len() > crate::MAX_AT_RESPONSE || raw.contains('\0') {
        return Err(invalid());
    }
    let mut lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if lines.first() == Some(&echo.as_str()) {
        lines.remove(0);
    }
    if lines.pop() != Some("OK") || lines.len() != 1 {
        return Err(invalid());
    }
    let fields: Vec<_> = lines[0]
        .strip_prefix("+QNWLOCK:")
        .ok_or_else(invalid)?
        .split(',')
        .map(str::trim)
        .collect();
    let expected = echo.strip_prefix("AT+QNWLOCK=").ok_or_else(invalid)?;
    if fields.first().copied() != Some(expected) {
        return Err(invalid());
    }
    let number = |s: &str, max: u64| -> Result<u64> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        s.parse::<u64>()
            .ok()
            .filter(|n| *n <= max)
            .ok_or_else(invalid)
    };
    if fields.len() == 2 && fields[1] == "0" {
        return Ok(
            json!({"rat":rat,"mode":0,"entries":[],"persistent":false,"scsRequired":rat=="NR"}),
        );
    }
    let (channel, pci, band, scs, mode) = if rat == "NR" {
        if fields.len() != 5 {
            return Err(PanelError::new("LOCK_VARIANT_UNSUPPORTED", 422));
        }
        let pci = number(fields[1], 1007)?;
        let channel = number(fields[2], 3279165)?;
        let khz = number(fields[3], 240)?;
        let scs = [15, 30, 60, 120, 240]
            .iter()
            .position(|v| *v == khz)
            .ok_or_else(invalid)?;
        let band = number(fields[4], 1024)?;
        if !radio::band_candidates("NR", channel as f64).contains(&format!("n{band}")) {
            return Err(invalid());
        }
        (channel, pci, json!(band), json!(scs), 2)
    } else {
        if fields.len() != 4 {
            return Err(invalid());
        }
        let kind = number(
            fields[1],
            if platform(model) == Some("lte") { 2 } else { 1 },
        )?;
        if kind == 0 {
            return Err(invalid());
        }
        let channel = number(fields[2], 262143)?;
        let pci = number(fields[3], 503)?;
        let frequency_only = platform(model) == Some("lte") && kind == 1;
        if frequency_only && pci != 0 {
            return Err(invalid());
        }
        let candidates = radio::band_candidates("LTE", channel as f64);
        let band = candidates
            .first()
            .filter(|_| candidates.len() == 1)
            .and_then(|b| b[1..].parse::<u64>().ok());
        (
            channel,
            pci,
            json!(band),
            Value::Null,
            if frequency_only { 1 } else { 2 },
        )
    };
    Ok(
        json!({"rat":rat,"mode":mode,"entries":[{"arfcn":channel,"pci":if mode==1{Value::Null}else{json!(pci)},"band":band,"scsType":scs}],"persistent":false,"scsRequired":rat=="NR"}),
    )
}
