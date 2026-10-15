//! 锁定操作公共校验及MT5700命令格式；其他平台由协议分派层处理。
use crate::{
    PanelError, Result,
    radio::{LTE_RANGES, NR_RANGES, band_candidates},
};
use serde_json::{Value, json};
fn bad() -> PanelError {
    PanelError::new("LOCK_INVALID", 400)
}
fn int(v: &Value, low: i64, high: i64) -> Result<i64> {
    v.as_i64()
        .filter(|v| *v >= low && *v <= high)
        .ok_or_else(bad)
}
fn exact(v: &Value, keys: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|map| map.len() == keys.len() && keys.iter().all(|key| map.contains_key(*key)))
}
pub fn presets(model: &str) -> Value {
    crate::band_inventory::presets(model)
}
pub fn validate_preset(model: &str, operation: &Value) -> Result<()> {
    if crate::band_locks::supported(model) && crate::band_locks::operation(operation) {
        crate::band_locks::validate(model, operation)?;
        return Ok(());
    }
    if crate::vendor_cell_lock::supported(model) && operation["mode"] == "cell" {
        crate::vendor_cell_lock::command(model, operation)?;
        return Ok(());
    }
    let inventory = presets(model);
    if inventory["lockProtocolSupported"] != true {
        return Err(PanelError::new("LOCK_UNSUPPORTED", 422));
    }
    if operation["mode"] == "band" && inventory["bandLockProtocolSupported"] != true {
        return Err(PanelError::new("LOCK_BAND_PROTOCOL_UNSUPPORTED", 422));
    }
    let rat = operation["rat"].as_str().ok_or_else(bad)?;
    let allowed = inventory[rat].as_array().ok_or_else(bad)?;
    if allowed.is_empty() {
        if crate::vendor_cell_lock::supported(model) {
            return Ok(());
        }
        return Err(PanelError::new("LOCK_UNSUPPORTED", 422));
    }
    let rows = operation["entries"].as_array().ok_or_else(bad)?;
    if rows.iter().any(|r| {
        if crate::lock_protocol::immediate(model)
            && operation["mode"] == "cell"
            && r["band"].is_null()
            && rat == "LTE"
            || crate::lock_protocol::unisoc(model)
                && operation["mode"] == "cell"
                && r["band"].is_null()
        {
            return !r["arfcn"].as_u64().is_some_and(|channel| {
                band_candidates(rat, channel as f64).iter().any(|b| {
                    b[1..]
                        .parse::<u64>()
                        .ok()
                        .is_some_and(|n| allowed.contains(&json!(n)))
                })
            });
        }
        !allowed.contains(&r["band"])
            || crate::lock_protocol::immediate(model)
                && operation["mode"] == "cell"
                && !r["arfcn"].as_u64().is_some_and(|channel| {
                    band_candidates(rat, channel as f64)
                        .iter()
                        .any(|b| b[1..].parse::<u64>().ok().is_some_and(|n| r["band"] == n))
                })
    }) {
        return Err(PanelError::new("LOCK_BAND_UNSUPPORTED", 422));
    }
    Ok(())
}
pub fn lock_command(operation: &Value) -> Result<String> {
    if !exact(operation, &["rat", "mode", "entries"]) {
        return Err(bad());
    }
    let rat = operation["rat"].as_str().ok_or_else(bad)?;
    let mode = operation["mode"].as_str().ok_or_else(bad)?;
    if !["NR", "LTE"].contains(&rat) || !["auto", "band", "cell"].contains(&mode) {
        return Err(bad());
    }
    let prefix = if rat == "NR" {
        "AT^NRFREQLOCK"
    } else {
        "AT^LTEFREQLOCK"
    };
    let rows = operation["entries"].as_array().ok_or_else(bad)?;
    if mode == "auto" {
        return if rows.is_empty() {
            Ok(format!("{prefix}=0"))
        } else {
            Err(bad())
        };
    }
    if rows.is_empty() || rows.len() > if mode == "band" { 20 } else { 4 } {
        return Err(bad());
    }
    let (mut bands, mut channels, mut scs, mut pcis) = (vec![], vec![], vec![], vec![]);
    for (index, row) in rows.iter().enumerate() {
        let keys = if mode == "band" {
            vec!["band"]
        } else if rat == "NR" {
            vec!["band", "arfcn", "scsType", "pci"]
        } else {
            vec!["band", "arfcn", "pci"]
        };
        if !exact(row, &keys) || rows[..index].contains(row) {
            return Err(bad());
        }
        let band = int(&row["band"], 1, 1024)?;
        let table = if rat == "NR" { NR_RANGES } else { LTE_RANGES };
        if !table.iter().any(|(b, _, _)| *b == band) || mode == "band" && bands.contains(&band) {
            return Err(bad());
        }
        bands.push(band);
        if mode == "cell" {
            let channel = int(&row["arfcn"], 0, if rat == "NR" { 3279165 } else { 262143 })?;
            if !band_candidates(rat, channel as f64)
                .contains(&format!("{}{band}", if rat == "NR" { "n" } else { "B" }))
            {
                return Err(bad());
            }
            channels.push(channel);
            pcis.push(int(&row["pci"], 0, if rat == "NR" { 1007 } else { 503 })?);
            if rat == "NR" {
                scs.push(int(&row["scsType"], 0, 4)?);
            }
        }
    }
    let fields = if mode == "band" {
        vec![bands]
    } else if rat == "NR" {
        vec![bands, channels, scs, pcis]
    } else {
        vec![bands, channels, pcis]
    };
    let mut command = format!(
        "{prefix}={}{}",
        if mode == "band" { "3,1," } else { "2,0," },
        rows.len()
    );
    for field in fields {
        command += &format!(
            ",\"{}\"",
            field
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    if command.len() > 190 {
        return Err(bad());
    }
    Ok(command)
}
pub fn parse_lock(raw: &str, rat: &str) -> Result<Value> {
    let error = || PanelError::new("LOCK_UNVERIFIED", 422);
    let prefix = match rat {
        "NR" => "^NRFREQLOCK:",
        "LTE" => "^LTEFREQLOCK:",
        _ => return Err(error()),
    };
    if raw.len() > crate::MAX_AT_RESPONSE || raw.contains('\0') {
        return Err(error());
    }
    let mut lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let echo = format!("AT{}?", prefix.trim_end_matches(':'));
    if lines.first() == Some(&echo.as_str()) {
        lines.remove(0);
    }
    if lines.last() != Some(&"OK")
        || lines.iter().filter(|l| **l == "OK").count() != 1
        || lines.iter().any(|l| l.contains("ERROR"))
    {
        return Err(error());
    }
    let headers: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with(prefix))
        .collect();
    if headers.len() != 1 || headers[0].0 != 0 {
        return Err(error());
    }
    let i = headers[0].0;
    let mode = lines[i][prefix.len()..]
        .trim()
        .parse::<i64>()
        .map_err(|_| error())?;
    if !(0..=3).contains(&mode) {
        return Err(error());
    }
    if mode == 0 {
        if lines.len() != 2 {
            return Err(error());
        }
        return Ok(json!({"rat":rat,"mode":0,"forbidFlag":1,"entries":[]}));
    }
    let counts: Vec<_> = lines
        .get(i + 1)
        .ok_or_else(error)?
        .split(',')
        .map(str::trim)
        .collect();
    if counts.len() != 2 {
        return Err(error());
    }
    let forbid = counts[0].parse::<i64>().map_err(|_| error())?;
    let count = counts[1].parse::<usize>().map_err(|_| error())?;
    if ![0, 1].contains(&forbid)
        || !(1..=20).contains(&count)
        || mode == 2 && count > 4
        || counts
            .iter()
            .any(|v| v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()))
        || lines.len() != i + 3 + count
        || lines.get(i + 2 + count) != Some(&"OK")
    {
        return Err(error());
    }
    let mut rows = vec![];
    for line in &lines[i + 2..i + 2 + count] {
        let expected = if mode == 3 {
            1
        } else if mode == 2 {
            if rat == "NR" { 4 } else { 3 }
        } else if rat == "NR" {
            3
        } else {
            2
        };
        let parts: Vec<_> = line.split(',').map(str::trim).collect();
        if parts.len() != expected {
            return Err(error());
        }
        let numbers = parts
            .iter()
            .map(|p| {
                if p.is_empty() || p.len() > 10 || !p.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(error());
                }
                p.parse::<i64>().map_err(|_| error())
            })
            .collect::<Result<Vec<_>>>()?;
        let bound = |v: i64, low: i64, high: i64| {
            if v >= low && v <= high {
                Ok(v)
            } else {
                Err(error())
            }
        };
        let mut row = json!({"band":bound(numbers[0],1,1024)?});
        if mode == 1 || mode == 2 {
            row["arfcn"] = json!(bound(
                numbers[1],
                0,
                if rat == "NR" { 3279165 } else { 262143 }
            )?);
            if rat == "NR" {
                row["scsType"] = json!(bound(numbers[2], 0, 4)?);
            }
            if mode == 2 {
                row["pci"] = json!(bound(
                    numbers[numbers.len() - 1],
                    0,
                    if rat == "NR" { 1007 } else { 503 }
                )?);
            }
        }
        if rows.contains(&row) {
            return Err(error());
        }
        rows.push(row);
    }
    Ok(json!({"rat":rat,"mode":mode,"forbidFlag":forbid,"entries":rows}))
}
pub fn write_accepted(raw: &str, command: &str) -> bool {
    if raw.len() > crate::MAX_AT_RESPONSE || raw.contains('\0') {
        return false;
    }
    let mut lines = raw.lines().map(str::trim).filter(|v| !v.is_empty());
    let first = lines.next();
    if first == Some(command) {
        lines.next() == Some("OK") && lines.next().is_none()
    } else {
        first == Some("OK") && lines.next().is_none()
    }
}
