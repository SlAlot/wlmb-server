//! 参考QModem平台小区锁定协议，校验候选参数并区分保存与射频生效。
use crate::{PanelError, Result, catalog, radio};
use serde_json::{Value, json};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Fibocom,
    Simcom,
    Gosuncn,
}
fn kind(model: &str) -> Option<Kind> {
    let p = catalog::identify(model);
    match (p["family"].as_str()?, p["platform"].as_str()?) {
        ("fibocom", _) => Some(Kind::Fibocom),
        ("simcom", "qualcomm") => Some(Kind::Simcom),
        ("gosuncn", "qualcomm") => Some(Kind::Gosuncn),
        _ => None,
    }
}
fn bad() -> PanelError {
    PanelError::new("LOCK_INVALID", 400)
}
fn unverified() -> PanelError {
    PanelError::new("LOCK_UNVERIFIED", 422)
}
pub fn supported(model: &str) -> bool {
    kind(model).is_some()
}
pub fn modem_reset(model: &str) -> bool {
    matches!(kind(model), Some(Kind::Fibocom | Kind::Gosuncn))
}
pub fn global_write(model: &str, mode: &str) -> bool {
    kind(model) == Some(Kind::Fibocom) || mode == "auto" && modem_reset(model)
}
pub fn query(model: &str, rat: &str) -> Result<String> {
    if !["NR", "LTE"].contains(&rat) {
        return Err(bad());
    }
    match kind(model) {
        Some(Kind::Fibocom) => Ok("AT+GTCELLLOCK?".into()),
        Some(Kind::Gosuncn) => Ok("AT+ZLOCKCELL?".into()),
        Some(Kind::Simcom) => Ok(if rat == "NR" {
            "AT+C5GCELLCFG?"
        } else {
            "AT+CCELLCFG?"
        }
        .into()),
        _ => Err(PanelError::new("LOCK_UNSUPPORTED", 422)),
    }
}
fn integer(s: &str, max: u64) -> Result<u64> {
    s.parse::<u64>()
        .ok()
        .filter(|n| *n <= max && !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
        .ok_or_else(unverified)
}
pub fn command(model: &str, op: &Value) -> Result<String> {
    let k = kind(model).ok_or_else(|| PanelError::new("LOCK_UNSUPPORTED", 422))?;
    if !op.as_object().is_some_and(|m| {
        m.len() == 3
            && ["rat", "mode", "entries"]
                .iter()
                .all(|key| m.contains_key(*key))
    }) {
        return Err(bad());
    }
    let rat = op["rat"]
        .as_str()
        .filter(|r| ["NR", "LTE"].contains(r))
        .ok_or_else(bad)?;
    let rows = op["entries"].as_array().ok_or_else(bad)?;
    if op["mode"] == "auto" && rows.is_empty() {
        return Ok(match (k, rat) {
            (Kind::Fibocom, _) => "AT+GTCELLLOCK=0",
            (Kind::Gosuncn, _) => "AT+ZLOCKCELL=0",
            (Kind::Simcom, "NR") => "AT+C5GCELLCFG=\"unlock\"",
            _ => "AT+CCELLCFG=0",
        }
        .into());
    }
    if op["mode"] != "cell" || rows.len() != 1 {
        return Err(bad());
    }
    let row = &rows[0];
    if !row.as_object().is_some_and(|m| {
        m.keys()
            .all(|k| ["band", "arfcn", "pci", "scsType", "bandCandidates"].contains(&k.as_str()))
    }) {
        return Err(bad());
    }
    let channel = row["arfcn"]
        .as_u64()
        .filter(|n| *n <= if rat == "NR" { 3279165 } else { 262143 })
        .ok_or_else(bad)?;
    let pci = row["pci"]
        .as_u64()
        .filter(|n| *n <= if rat == "NR" { 1007 } else { 503 })
        .ok_or_else(bad)?;
    if rat == "NR" {
        let band = row["band"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= 1024)
            .ok_or_else(bad)?;
        let scs = row["scsType"]
            .as_u64()
            .filter(|n| *n <= 4)
            .ok_or_else(bad)?;
        if !radio::band_candidates(rat, channel as f64).contains(&format!("n{band}")) {
            return Err(bad());
        }
        if k == Kind::Gosuncn && scs > 3 {
            return Err(bad());
        }
        Ok(if k == Kind::Gosuncn {
            format!("AT+ZLOCKCELL=1,2,{channel},{pci},{scs},{band}")
        } else if k == Kind::Fibocom {
            format!("AT+GTCELLLOCK=1,1,0,{channel},{pci},{scs},50{band}")
        } else {
            format!("AT+C5GCELLCFG=\"pci\",{pci},{channel},{scs},{band}")
        })
    } else {
        Ok(if k == Kind::Gosuncn {
            format!("AT+ZLOCKCELL=1,1,{channel},{pci}")
        } else if k == Kind::Fibocom {
            format!("AT+GTCELLLOCK=1,0,0,{channel},{pci}")
        } else {
            format!("AT+CCELLCFG=1,{pci},{channel}")
        })
    }
}
pub fn parse(model: &str, raw: &str, rat: &str) -> Result<Value> {
    let k = kind(model).ok_or_else(unverified)?;
    let query = query(model, rat)?;
    let lines = crate::band_locks::payload(raw, &query)?;
    if k == Kind::Gosuncn {
        return gosuncn(lines, rat);
    }
    if lines.len() != 1 {
        return Err(unverified());
    }
    let prefix = match (k, rat) {
        (Kind::Fibocom, _) => "+GTCELLLOCK:",
        (Kind::Simcom, "NR") => "+C5GCELLCFG:",
        _ => "+CCELLCFG:",
    };
    let p: Vec<_> = lines[0]
        .strip_prefix(prefix)
        .ok_or_else(unverified)?
        .split(',')
        .map(str::trim)
        .collect();
    let empty = || json!({"rat":rat,"mode":0,"entries":[],"persistent":false});
    if p == ["0"] || k == Kind::Simcom && rat == "NR" && p == ["\"unlock\""] {
        return Ok(empty());
    }
    let (channel, pci, band, scs, mode) = if k == Kind::Fibocom {
        if p.len() < 4 || p[0] != "1" || !["0", "1"].contains(&p[1]) || !["0", "1"].contains(&p[2])
        {
            return Err(unverified());
        }
        let nr = p[1] == "1";
        let ch = integer(p[3], if nr { 3279165 } else { 262143 })?;
        let frequency_only = p[2] == "1";
        let expected = if frequency_only {
            4
        } else if nr {
            7
        } else {
            5
        };
        if p.len() != expected {
            return Err(unverified());
        }
        let pci = if frequency_only {
            Value::Null
        } else {
            json!(integer(p[4], if nr { 1007 } else { 503 })?)
        };
        let (band, scs) = if nr && !frequency_only {
            (
                json!(integer(
                    p[6].strip_prefix("50").ok_or_else(unverified)?,
                    1024
                )?),
                json!(integer(p[5], 4)?),
            )
        } else {
            (Value::Null, Value::Null)
        };
        if nr != (rat == "NR") {
            return Ok(empty());
        }
        (ch, pci, band, scs, if frequency_only { 1 } else { 2 })
    } else if rat == "NR" {
        if p.len() != 5 || p[0] != "\"pci\"" {
            return Err(unverified());
        }
        (
            integer(p[2], 3279165)?,
            json!(integer(p[1], 1007)?),
            json!(integer(p[4], 1024)?),
            json!(integer(p[3], 4)?),
            2,
        )
    } else {
        if p.len() != 2 {
            return Err(unverified());
        }
        (
            integer(p[1], 262143)?,
            json!(integer(p[0], 503)?),
            Value::Null,
            Value::Null,
            2,
        )
    };
    if rat == "NR"
        && !band.is_null()
        && !radio::band_candidates(rat, channel as f64).contains(&format!("n{band}"))
    {
        return Err(unverified());
    }
    Ok(
        json!({"rat":rat,"mode":mode,"entries":[{"arfcn":channel,"pci":pci,"band":band,"scsType":scs}],"persistent":false,"globalUnlock":k==Kind::Fibocom,"scsRequired":rat=="NR"}),
    )
}
fn gosuncn(lines: Vec<&str>, rat: &str) -> Result<Value> {
    use std::collections::BTreeMap;
    let mut groups = BTreeMap::new();
    for line in lines {
        let line = line.strip_prefix("+ZLOCKCELL:").unwrap_or(line).trim();
        let p: Vec<_> = line.split(',').map(str::trim).collect();
        let group = match p.first().copied() {
            Some("LTE") => "LTE",
            Some("NR5G") => "NR",
            _ => return Err(unverified()),
        };
        let mut fields = BTreeMap::new();
        for part in &p[1..] {
            let (key, v) = part.split_once(':').ok_or_else(unverified)?;
            if fields.insert(key.trim(), v.trim()).is_some() {
                return Err(unverified());
            }
        }
        let (_, status) = p
            .last()
            .ok_or_else(unverified)?
            .split_once(':')
            .ok_or_else(unverified)?;
        if !["on", "off"].contains(&status.trim()) {
            return Err(unverified());
        }
        let value = if status.trim() == "off" {
            json!({"rat":group,"mode":0,"entries":[],"globalUnlock":true})
        } else {
            let get = |key| fields.get(key).copied().ok_or_else(unverified);
            let channel = integer(
                get(if group == "LTE" {
                    "earfcn"
                } else {
                    "nr5g_chanel"
                })?,
                if group == "LTE" { 262143 } else { 3279165 },
            )?;
            let pci = integer(get("pci")?, if group == "LTE" { 503 } else { 1007 })?;
            let (band, scs) = if group == "NR" {
                (
                    json!(integer(get("band")?, 1024)?),
                    json!(integer(get("scs")?, 3)?),
                )
            } else {
                (Value::Null, Value::Null)
            };
            if group == "NR"
                && !radio::band_candidates("NR", channel as f64).contains(&format!("n{band}"))
            {
                return Err(unverified());
            }
            json!({"rat":group,"mode":2,"entries":[{"arfcn":channel,"pci":pci,"band":band,"scsType":scs}],"globalUnlock":true,"persistent":false})
        };
        if groups.insert(group, value).is_some() {
            return Err(unverified());
        }
    }
    groups.remove(rat).ok_or_else(unverified)
}
