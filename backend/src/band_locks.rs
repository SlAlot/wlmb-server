//! 参考QModem各平台频段协议，统一频段状态、写入构造和保存回读校验。
use crate::{PanelError, Result, band_inventory, catalog};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
mod templates;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Quectel,
    Simcom,
    Qcfg,
    FoxconnPlus,
    FoxconnCaret,
    Thales,
    Gosuncn,
    GosuncnLte,
    Fibocom,
    Telit,
    Air724,
    SimcomLte,
    Neoway,
}
fn kind(model: &str) -> Option<Kind> {
    let p = catalog::identify(model);
    match (p["family"].as_str()?, p["platform"].as_str()?) {
        ("quectel", "qualcomm" | "lte12" | "unisoc") => Some(Kind::Quectel),
        ("quectel", "lte") => Some(Kind::Qcfg),
        ("simcom", "qualcomm" | "asrmicro") => Some(Kind::Simcom),
        ("foxconn", _) if ["t99w368", "t99w640"].contains(&p["model"].as_str()?) => {
            Some(Kind::FoxconnPlus)
        }
        ("foxconn", _) => Some(Kind::FoxconnCaret),
        ("thales", _) => Some(Kind::Thales),
        ("gosuncn", "qualcomm") => Some(Kind::Gosuncn),
        ("gosuncn", "lte") => Some(Kind::GosuncnLte),
        ("fibocom", _) => Some(Kind::Fibocom),
        ("telit", _) => Some(Kind::Telit),
        ("openluat", "unisoc") => Some(Kind::Air724),
        ("simcom", "lte") => Some(Kind::SimcomLte),
        ("neoway", "unisoc") => Some(Kind::Neoway),
        _ => None,
    }
}
fn invalid() -> PanelError {
    PanelError::new("LOCK_UNVERIFIED", 422)
}
pub fn supported(model: &str) -> bool {
    kind(model).is_some() || templates::supported(model)
}
pub fn operation(op: &Value) -> bool {
    op["mode"] == "band" || op["target"] == "band"
}
pub fn modem_reset(model: &str) -> bool {
    matches!(kind(model), Some(Kind::Gosuncn | Kind::GosuncnLte))
}
pub fn planes(model: &str) -> Vec<&'static str> {
    let separate = matches!(
        kind(model),
        Some(Kind::Quectel | Kind::Simcom | Kind::Telit)
    ) || templates::supported(model);
    let lte_only = catalog::platform(model) == "lte" || catalog::platform(model) == "lte12";
    ["LTE", "NR", "NR_NSA"]
        .into_iter()
        .filter(|rat| (*rat != "NR_NSA" || separate) && (*rat == "LTE" || !lte_only))
        .collect()
}
fn selector(rat: &str) -> Result<&'static str> {
    match rat {
        "LTE" => Ok("lte_band"),
        "NR" => Ok("nr5g_band"),
        "NR_NSA" => Ok("nsa_nr5g_band"),
        _ => Err(invalid()),
    }
}
pub fn queries(model: &str) -> Vec<String> {
    if templates::supported(model) {
        return vec!["AT!BAND?".into(), "AT!BAND=?".into()];
    }
    match kind(model) {
        Some(Kind::Quectel | Kind::Simcom) => planes(model)
            .iter()
            .filter_map(|rat| selector(rat).ok())
            .map(|s| {
                format!(
                    "AT+{}=\"{s}\"",
                    if kind(model) == Some(Kind::Quectel) {
                        "QNWPREFCFG"
                    } else {
                        "CSYSSEL"
                    }
                )
            })
            .collect(),
        Some(Kind::Qcfg) => vec!["AT+QCFG=\"band\"".into()],
        Some(Kind::FoxconnCaret) => vec!["AT^BAND_PREF?".into()],
        Some(Kind::FoxconnPlus | Kind::Thales) => vec!["AT+BAND_PREF?".into()],
        Some(Kind::Gosuncn | Kind::GosuncnLte) => vec!["AT+ZBAND?".into(), "AT+ZBAND=?".into()],
        Some(Kind::Fibocom) => vec!["AT+GTACT?".into(), "AT+GTACT=?".into()],
        Some(Kind::Telit) => vec!["AT#BND?".into(), "AT#BND=?".into()],
        Some(Kind::Air724) => vec!["AT*BAND?".into()],
        Some(Kind::SimcomLte) => vec!["AT+CNBP?".into()],
        Some(Kind::Neoway) => vec!["AT+NWSETBAND?".into(), "AT+NWSETBAND=?".into()],
        _ => vec![],
    }
}
pub(crate) fn payload<'a>(raw: &'a str, command: &str) -> Result<Vec<&'a str>> {
    if raw.len() > crate::MAX_AT_RESPONSE || raw.contains('\0') {
        return Err(invalid());
    }
    let mut lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.first() == Some(&command) {
        lines.remove(0);
    }
    if lines.pop() != Some("OK")
        || lines.is_empty()
        || lines
            .iter()
            .any(|l| *l == "OK" || l.starts_with("AT") || l.contains("ERROR"))
    {
        return Err(invalid());
    }
    Ok(lines)
}
fn list(raw: &str, separator: char) -> Result<Vec<u64>> {
    if raw.trim().is_empty() {
        return Ok(vec![]);
    }
    let mut out = BTreeSet::new();
    for part in raw.split(separator) {
        let part = part.trim();
        let band = part
            .parse::<u64>()
            .ok()
            .filter(|n| (1..=1024).contains(n) && part.bytes().all(|c| c.is_ascii_digit()))
            .ok_or_else(invalid)?;
        if !out.insert(band) || out.len() > 128 {
            return Err(invalid());
        }
    }
    Ok(out.into_iter().collect())
}
fn allowed(model: &str, rat: &str) -> Result<Vec<u64>> {
    let inv = band_inventory::presets(model);
    let field = if rat == "NR_NSA" {
        "NRNSA"
    } else if rat == "NR"
        && matches!(
            kind(model),
            Some(Kind::Quectel | Kind::Simcom | Kind::Telit)
        )
        || rat == "NR" && templates::supported(model)
    {
        "NRSA"
    } else {
        rat
    };
    inv[field]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|v| v.as_u64().ok_or_else(invalid))
        .collect()
}
fn group(model: &str, rat: &str, bands: Vec<u64>) -> Result<Value> {
    let all = allowed(model, rat)?;
    let auto = !all.is_empty() && bands.len() == all.len() && all.iter().all(|b| bands.contains(b));
    Ok(
        json!({"rat":rat,"mode":if auto {0}else{3},"entries":bands.iter().map(|b|json!({"band":b})).collect::<Vec<_>>(),"selectedBands":bands,"availableBands":all,"target":"band"}),
    )
}
fn hex_bands(s: &str) -> Result<Vec<u64>> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if s.is_empty() || s.len() > 32 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let mask = u128::from_str_radix(s, 16).map_err(|_| invalid())?;
    Ok((1..=128)
        .filter(|b| mask & (1u128 << (b - 1)) != 0)
        .collect())
}
pub fn parse(model: &str, replies: &BTreeMap<String, String>) -> Result<Value> {
    if templates::supported(model) {
        return templates::parse(model, replies);
    }
    let k = kind(model).ok_or_else(invalid)?;
    let mut groups = serde_json::Map::new();
    let mut wire = json!({});
    for query in queries(model) {
        let Some(raw) = replies.get(&query) else {
            continue;
        };
        let lines = payload(raw, &query)?;
        match k {
            Kind::Quectel | Kind::Simcom => {
                if lines.len() != 1 {
                    return Err(invalid());
                }
                let prefix = if k == Kind::Quectel {
                    "+QNWPREFCFG:"
                } else {
                    "+CSYSSEL:"
                };
                let (s, bands) = lines[0]
                    .strip_prefix(prefix)
                    .ok_or_else(invalid)?
                    .trim()
                    .split_once(',')
                    .ok_or_else(invalid)?;
                let rat = planes(model)
                    .into_iter()
                    .find(|rat| {
                        selector(rat)
                            .ok()
                            .is_some_and(|v| s.trim() == format!("\"{v}\""))
                    })
                    .ok_or_else(invalid)?;
                let bands = list(bands, ':')?;
                groups.insert(rat.into(), group(model, rat, bands)?);
            }
            Kind::Qcfg => {
                if lines.len() != 1 {
                    return Err(invalid());
                }
                let p: Vec<_> = lines[0]
                    .strip_prefix("+QCFG:")
                    .ok_or_else(invalid)?
                    .split(',')
                    .map(str::trim)
                    .collect();
                if p.len() != 4 || p[0] != "\"band\"" {
                    return Err(invalid());
                }
                for mask in &p[1..] {
                    hex_bands(mask)?;
                }
                wire["masks"] = json!(p[1..]);
                groups.insert("LTE".into(), group(model, "LTE", hex_bands(p[2])?)?);
            }
            Kind::FoxconnPlus | Kind::FoxconnCaret | Kind::Thales => {
                let mut found = BTreeMap::<String, Vec<u64>>::new();
                for line in lines {
                    let line = line
                        .strip_prefix("+BAND_PREF:")
                        .or_else(|| line.strip_prefix("^BAND_PREF:"))
                        .unwrap_or(line)
                        .trim();
                    let (label, bands) = line.split_once(':').ok_or_else(invalid)?;
                    let label = label
                        .trim()
                        .trim_start_matches("+BAND_PREF")
                        .trim_start_matches("^BAND_PREF")
                        .trim();
                    let (rat, enabled) = label.split_once(',').ok_or_else(invalid)?;
                    if !["WCDMA", "LTE", "NR5G_NSA", "NR5G_SA", "NR5G"].contains(&rat.trim())
                        || !["Enable Bands", "Disable Bands"].contains(&enabled.trim())
                    {
                        return Err(invalid());
                    }
                    let key = format!("{},{}", rat.trim(), enabled.trim());
                    if found.insert(key, list(bands, ',')?).is_some() {
                        return Err(invalid());
                    }
                }
                for rat in planes(model) {
                    let names = if rat == "LTE" {
                        vec!["LTE"]
                    } else {
                        vec!["NR5G_NSA", "NR5G_SA"]
                    };
                    let mut active = BTreeSet::new();
                    let mut available = BTreeSet::new();
                    for name in names {
                        let a = found
                            .get(&format!("{name},Enable Bands"))
                            .ok_or_else(invalid)?;
                        let d = found
                            .get(&format!("{name},Disable Bands"))
                            .ok_or_else(invalid)?;
                        if a.iter().any(|b| d.contains(b)) {
                            return Err(invalid());
                        }
                        active.extend(a.iter().copied());
                        available.extend(a.iter().chain(d.iter()).copied());
                    }
                    let mut value = group(model, rat, active.into_iter().collect())?;
                    value["availableBands"] = json!(available);
                    value["availableSource"] = json!("firmware");
                    groups.insert(rat.into(), value);
                }
            }
            Kind::Gosuncn | Kind::GosuncnLte => {
                for line in lines {
                    let line = line.strip_prefix("+ZBAND:").unwrap_or(line).trim();
                    let (name, bands) = line.split_once(':').ok_or_else(invalid)?;
                    let rat = match name.trim() {
                        "LTE" => "LTE",
                        "NR5G" => "NR",
                        "WCDMA" | "GSM" | "TDSCDMA" => continue,
                        _ => return Err(invalid()),
                    };
                    if !planes(model).contains(&rat) {
                        return Err(invalid());
                    }
                    if query.ends_with("=?") {
                        wire["available"][rat] = json!(list(
                            bands
                                .trim()
                                .strip_prefix('(')
                                .and_then(|v| v.strip_suffix(')'))
                                .unwrap_or(bands),
                            ','
                        )?);
                        continue;
                    }
                    if groups
                        .insert(rat.into(), group(model, rat, list(bands, ',')?)?)
                        .is_some()
                    {
                        return Err(invalid());
                    }
                }
            }
            Kind::Fibocom => {
                if lines.len() != 1 {
                    return Err(invalid());
                }
                let raw = lines[0].strip_prefix("+GTACT:").ok_or_else(invalid)?.trim();
                if query.ends_with("=?") {
                    let mut parts = Vec::new();
                    let mut rest = raw;
                    while let Some(tail) = rest.strip_prefix('(') {
                        let (part, next) = tail.split_once(')').ok_or_else(invalid)?;
                        parts.push(part);
                        rest = next.trim();
                        if rest.is_empty() {
                            break;
                        }
                        rest = rest.strip_prefix(',').ok_or_else(invalid)?.trim();
                        if parts.len() > 16 {
                            return Err(invalid());
                        }
                    }
                    if !rest.is_empty() || parts.len() < 9 {
                        return Err(invalid());
                    }
                    wire["availableCodes"] = json!([
                        list(parts[4], ',')?,
                        code_list(parts[5])?,
                        code_list(parts[8])?
                    ]);
                } else {
                    let p: Vec<_> = raw.split(',').map(str::trim).collect();
                    if p.len() < 3
                        || !["1", "2", "4", "10", "14", "16", "17", "20"].contains(&p[0])
                        || p[1..3]
                            .iter()
                            .any(|s| !s.is_empty() && s.parse::<u8>().is_err())
                    {
                        return Err(invalid());
                    }
                    wire["fields"] = json!(p[..3]);
                    wire["codes"] = json!(code_list(&p[3..].join(","))?);
                }
            }
            Kind::Telit => {
                if lines.len() != 1 {
                    return Err(invalid());
                }
                let raw = lines[0].strip_prefix("#BND:").ok_or_else(invalid)?.trim();
                if query.ends_with("=?") {
                    let parts = parentheses(raw)?;
                    if parts.len() != 8 {
                        return Err(invalid());
                    }
                    for part in &parts[2..8] {
                        hex_bands(part)?;
                    }
                    wire["availableMasks"] = json!(parts[2..]);
                    continue;
                }
                let p: Vec<_> = raw.split(',').map(str::trim).collect();
                if p.len() != 8 || p[0] != "0" || p[1] != "22" {
                    return Err(invalid());
                }
                for mask in &p[2..] {
                    hex_bands(mask)?;
                }
                wire["fields"] = json!(p);
                groups.insert("LTE".into(), group(model, "LTE", hex_bands(p[2])?)?);
                for (rat, low, high) in [("NR_NSA", 4, 5), ("NR", 6, 7)] {
                    let mut bands = hex_bands(p[low])?;
                    if bands.iter().any(|b| *b > 64) {
                        return Err(invalid());
                    }
                    let upper = hex_bands(p[high])?;
                    if upper.iter().any(|b| *b > 64) {
                        return Err(invalid());
                    }
                    bands.extend(upper.into_iter().map(|b| b + 64));
                    groups.insert(rat.into(), group(model, rat, bands)?);
                }
            }
            Kind::Air724 => {
                if lines.len() != 1 {
                    return Err(invalid());
                }
                let p: Vec<_> = lines[0]
                    .strip_prefix("*BAND:")
                    .ok_or_else(invalid)?
                    .split(',')
                    .map(str::trim)
                    .collect();
                if p.len() != 8 {
                    return Err(invalid());
                }
                let values = p.iter().map(|v| decimal(v)).collect::<Result<Vec<_>>>()?;
                if values[0] > 15 || values[3] > u32::MAX as u64 || values[4] > u32::MAX as u64 {
                    return Err(invalid());
                }
                wire["fields"] = json!(values);
                let all = allowed(model, "LTE")?;
                let bands = if values[0] == 5 {
                    all.into_iter()
                        .filter(|b| {
                            if *b > 32 {
                                values[3] & (1 << (*b - 33)) != 0
                            } else {
                                values[4] & (1 << (*b - 1)) != 0
                            }
                        })
                        .collect()
                } else {
                    all
                };
                groups.insert("LTE".into(), group(model, "LTE", bands)?);
            }
            Kind::SimcomLte => {
                if lines.len() != 1 {
                    return Err(invalid());
                }
                let p: Vec<_> = lines[0]
                    .strip_prefix("+CNBP:")
                    .ok_or_else(invalid)?
                    .split(',')
                    .map(str::trim)
                    .collect();
                if ![3, 4].contains(&p.len()) || p.len() == 4 && !["0", "1"].contains(&p[3]) {
                    return Err(invalid());
                }
                hex_bands(p[0])?;
                let mut bands = hex_bands(p[1])?;
                if bands.iter().any(|b| *b > 32) {
                    return Err(PanelError::new("LOCK_VARIANT_UNSUPPORTED", 422));
                }
                let upper = hex_bands(p[2])?;
                if upper.iter().any(|b| *b > 96) {
                    return Err(invalid());
                }
                bands.extend(upper.into_iter().map(|b| b + 32));
                wire["fields"] = json!(p);
                groups.insert("LTE".into(), group(model, "LTE", bands)?);
            }
            Kind::Neoway => {
                if query.ends_with("=?") {
                    let mut available = BTreeMap::<String, BTreeSet<u64>>::new();
                    for line in lines {
                        let row = line.strip_prefix("+NWSETBAND:").ok_or_else(invalid)?;
                        let p: Vec<_> = row.split(',').map(str::trim).collect();
                        if p.len() < 2 {
                            return Err(invalid());
                        }
                        decimal(p[0])?;
                        for b in &p[1..] {
                            let (rat, n) = if let Some(n) = b.strip_prefix("WB") {
                                ("UMTS", n)
                            } else if let Some(n) = b.strip_prefix('B') {
                                ("LTE", n)
                            } else if let Some(n) = b.strip_prefix('N') {
                                ("NR", n)
                            } else {
                                return Err(invalid());
                            };
                            let number = decimal(n)?;
                            if !(1..=1024).contains(&number) {
                                return Err(invalid());
                            }
                            available.entry(rat.into()).or_default().insert(number);
                        }
                    }
                    wire["available"] = json!(available);
                    continue;
                }
                let header = lines[0]
                    .strip_prefix("+NWSETBAND:")
                    .ok_or_else(invalid)?
                    .trim();
                let count = decimal(header)?;
                wire["automatic"] = json!(count == 0);
                if count > 3 || lines.len() != count as usize + 1 {
                    return Err(invalid());
                }
                let mut selected = BTreeMap::new();
                for line in &lines[1..] {
                    let p: Vec<_> = line.split(',').map(str::trim).collect();
                    if p.len() < 3 || decimal(p[1])? as usize != p.len() - 2 {
                        return Err(invalid());
                    }
                    let (rat, prefix) = match p[0] {
                        "2" => ("UMTS", "WB"),
                        "3" => ("LTE", "B"),
                        "6" => ("NR", "N"),
                        _ => return Err(invalid()),
                    };
                    let bands = list(
                        &p[2..]
                            .iter()
                            .map(|v| v.strip_prefix(prefix).ok_or_else(invalid))
                            .collect::<Result<Vec<_>>>()?
                            .join(","),
                        ',',
                    )?;
                    if selected.insert(rat, bands).is_some() {
                        return Err(invalid());
                    }
                }
                for rat in planes(model) {
                    let bands = if count == 0 {
                        allowed(model, rat)?
                    } else {
                        selected.remove(rat).ok_or_else(invalid)?
                    };
                    groups.insert(rat.into(), group(model, rat, bands)?);
                }
            }
        }
    }
    if k == Kind::Fibocom {
        let fields = wire["fields"]
            .as_array()
            .filter(|a| a.len() == 3)
            .ok_or_else(invalid)?;
        let all = wire["availableCodes"]
            .as_array()
            .filter(|a| a.len() == 3)
            .cloned()
            .ok_or_else(invalid)?;
        let mode = fields[0].as_str().ok_or_else(invalid)?;
        let mut codes = wire["codes"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|v| v.as_u64().ok_or_else(invalid))
            .collect::<Result<Vec<_>>>()?;
        if codes.is_empty() {
            for (i, active) in [
                (["1", "4", "10", "16", "20"].contains(&mode)),
                (["2", "4", "10", "17", "20"].contains(&mode)),
                (["14", "16", "17", "10", "20"].contains(&mode)),
            ]
            .into_iter()
            .enumerate()
            {
                if active {
                    codes.extend(
                        all[i]
                            .as_array()
                            .ok_or_else(invalid)?
                            .iter()
                            .map(|v| v.as_u64().ok_or_else(invalid))
                            .collect::<Result<Vec<_>>>()?,
                    );
                }
            }
        }
        wire["codes"] = json!(codes);
        for rat in planes(model) {
            let bands = codes.iter().filter_map(|c| decode_code(*c, rat)).collect();
            let mut value = group(model, rat, bands)?;
            value["availableBands"] = json!(
                all[if rat == "LTE" { 1 } else { 2 }]
                    .as_array()
                    .ok_or_else(invalid)?
                    .iter()
                    .filter_map(Value::as_u64)
                    .filter_map(|c| decode_code(c, rat))
                    .collect::<Vec<_>>()
            );
            value["availableSource"] = json!("firmware");
            groups.insert(rat.into(), value);
        }
    }
    if k == Kind::Telit {
        let masks = wire["availableMasks"]
            .as_array()
            .filter(|a| a.len() == 6)
            .ok_or_else(invalid)?;
        for (rat, i) in [("LTE", 0), ("NR_NSA", 2), ("NR", 4)] {
            let mut all = hex_bands(masks[i].as_str().ok_or_else(invalid)?)?;
            if rat != "LTE" {
                let upper = hex_bands(masks[i + 1].as_str().ok_or_else(invalid)?)?;
                if all.iter().chain(upper.iter()).any(|b| *b > 64) {
                    return Err(invalid());
                }
                all.extend(upper.into_iter().map(|b| b + 64));
            }
            groups.get_mut(rat).ok_or_else(invalid)?["availableBands"] = json!(all);
            groups.get_mut(rat).ok_or_else(invalid)?["availableSource"] = json!("firmware");
        }
    }
    if k == Kind::Neoway {
        for rat in planes(model) {
            groups.get_mut(rat).ok_or_else(invalid)?["availableBands"] =
                wire["available"][rat].clone();
            groups.get_mut(rat).ok_or_else(invalid)?["availableSource"] = json!("firmware");
            if wire["automatic"] == true {
                let a = wire["available"][rat].as_array().ok_or_else(invalid)?;
                let g = groups.get_mut(rat).ok_or_else(invalid)?;
                g["selectedBands"] = json!(a);
                g["mode"] = json!(0);
                g["entries"] = json!(a.iter().map(|b| json!({"band":b})).collect::<Vec<_>>());
            }
        }
    }
    if matches!(k, Kind::Gosuncn | Kind::GosuncnLte) {
        for (rat, group) in groups.iter_mut() {
            if let Some(all) = wire["available"][rat].as_array().filter(|a| !a.is_empty()) {
                group["availableBands"] = json!(all);
                group["availableSource"] = json!("firmware");
            }
        }
    }
    if groups.is_empty() {
        return Err(invalid());
    }
    for g in groups.values_mut() {
        let all = g["availableBands"].as_array().ok_or_else(invalid)?;
        let selected = g["selectedBands"].as_array().ok_or_else(invalid)?;
        if !all.is_empty() {
            if selected.iter().any(|b| !all.contains(b)) {
                return Err(invalid());
            }
            let auto = selected.len() == all.len() && all.iter().all(|b| selected.contains(b));
            g["mode"] = json!(if auto { 0 } else { 3 });
        }
    }
    Ok(json!({"groups":groups,"wire":wire}))
}
pub fn validate(model: &str, op: &Value) -> Result<Vec<u64>> {
    let bad = || PanelError::new("LOCK_INVALID", 400);
    if !supported(model) || !operation(op) {
        return Err(PanelError::new("LOCK_BAND_PROTOCOL_UNSUPPORTED", 422));
    }
    if !op.as_object().is_some_and(|m| {
        m.keys()
            .all(|k| ["rat", "mode", "entries", "target"].contains(&k.as_str()))
            && [3, 4].contains(&m.len())
    }) || !["band", "auto"].contains(&op["mode"].as_str().unwrap_or(""))
        || op.get("target").is_some_and(|t| t != "band")
    {
        return Err(bad());
    }
    let rat = op["rat"]
        .as_str()
        .filter(|r| planes(model).contains(r))
        .ok_or_else(bad)?;
    let rows = op["entries"].as_array().ok_or_else(bad)?;
    if (op["mode"] == "auto") != rows.is_empty() || rows.len() > 128 {
        return Err(bad());
    }
    let all = allowed(model, rat)?;
    let mut out = BTreeSet::new();
    for row in rows {
        let band = row["band"].as_u64().ok_or_else(bad)?;
        if !row.as_object().is_some_and(|m| m.len() == 1)
            || !(1..=1024).contains(&band)
            || !out.insert(band)
        {
            return Err(bad());
        }
    }
    Ok(if rows.is_empty() {
        all
    } else {
        out.into_iter().collect()
    })
}
pub fn commands(model: &str, op: &Value, state: &Value) -> Result<Vec<String>> {
    let mut bands = validate(model, op)?;
    let rat = op["rat"].as_str().ok_or_else(invalid)?;
    if state["groups"].get(rat).is_none() {
        return Err(invalid());
    }
    let all = state["groups"][rat]["availableBands"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(invalid)?;
    let available = all
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|n| (1..=1024).contains(n))
                .ok_or_else(invalid)
        })
        .collect::<Result<Vec<_>>>()?;
    if op["mode"] == "auto" {
        bands = available.clone();
    }
    if bands.iter().any(|b| !available.contains(b)) {
        return Err(PanelError::new("LOCK_BAND_UNSUPPORTED", 422));
    }
    if templates::supported(model) {
        return templates::commands(op, state, &bands);
    }
    let k = kind(model).ok_or_else(invalid)?;
    let join = |separator: &str| {
        bands
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(separator)
    };
    let command = match k {
        Kind::Quectel | Kind::Simcom => format!(
            "AT+{}=\"{}\",{}",
            if k == Kind::Quectel {
                "QNWPREFCFG"
            } else {
                "CSYSSEL"
            },
            selector(rat)?,
            join(":")
        ),
        Kind::Qcfg => {
            let masks = state["wire"]["masks"]
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or_else(invalid)?;
            let masks = masks
                .iter()
                .map(|m| m.as_str().ok_or_else(invalid))
                .collect::<Result<Vec<_>>>()?;
            for mask in &masks {
                hex_bands(mask)?;
            }
            let mut mask = 0u128;
            for band in bands {
                if band > 128 {
                    return Err(invalid());
                }
                mask |= 1u128 << (band - 1);
            }
            format!("AT+QCFG=\"band\",{},0x{mask:X},{}", masks[0], masks[2])
        }
        Kind::FoxconnCaret | Kind::FoxconnPlus | Kind::Thales => format!(
            "AT{}BAND_PREF={},2,{}",
            if k == Kind::FoxconnCaret { "^" } else { "+" },
            if rat == "LTE" { "LTE" } else { "NR5G" },
            join(",")
        ),
        Kind::Gosuncn => {
            if op["mode"] == "auto" {
                return Ok(vec!["AT+ZBAND=0".into()]);
            }
            if bands.len() > 10 {
                return Err(PanelError::new("LOCK_TOO_MANY_BANDS", 422));
            }
            format!(
                "AT+ZBAND={},{},{}",
                if rat == "LTE" { 1 } else { 5 },
                bands.len(),
                join(",")
            )
        }
        Kind::GosuncnLte => {
            if op["mode"] == "auto" {
                return Ok(vec!["AT+ZBAND=all,all,all,all".into()]);
            }
            let mut mask = 0u128;
            for band in bands {
                if band > 128 {
                    return Err(invalid());
                }
                mask |= 1u128 << (band - 1);
            }
            format!("AT+ZBAND=all,all,all,{mask:X}")
        }
        Kind::Fibocom => {
            let p = state["wire"]["fields"]
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or_else(invalid)?;
            let fields = p
                .iter()
                .map(|v| v.as_str().ok_or_else(invalid))
                .collect::<Result<Vec<_>>>()?;
            if !["1", "2", "4", "10", "14", "16", "17", "20"].contains(&fields[0])
                || fields[1..]
                    .iter()
                    .any(|s| !s.is_empty() && s.parse::<u8>().is_err())
            {
                return Err(invalid());
            }
            let all = state["wire"]["availableCodes"]
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or_else(invalid)?;
            let available = all[if rat == "LTE" { 1 } else { 2 }]
                .as_array()
                .ok_or_else(invalid)?;
            let mut codes = state["wire"]["codes"]
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(|c| c.as_u64().ok_or_else(invalid))
                .collect::<Result<Vec<_>>>()?;
            codes.retain(|c| decode_code(*c, rat).is_none());
            for band in bands {
                let matched: Vec<_> = available
                    .iter()
                    .filter_map(Value::as_u64)
                    .filter(|c| decode_code(*c, rat) == Some(band))
                    .collect();
                if matched.len() != 1 {
                    return Err(invalid());
                }
                codes.push(matched[0]);
            }
            codes.sort_unstable();
            codes.dedup();
            format!(
                "AT+GTACT={},{},{},{}",
                fields[0],
                fields[1],
                fields[2],
                codes
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Kind::Telit => {
            let p = state["wire"]["fields"]
                .as_array()
                .filter(|a| a.len() == 8)
                .ok_or_else(invalid)?;
            let mut fields = p
                .iter()
                .map(|v| v.as_str().map(str::to_owned).ok_or_else(invalid))
                .collect::<Result<Vec<_>>>()?;
            if fields[..2] != ["0", "22"] {
                return Err(invalid());
            }
            for mask in &fields[2..] {
                hex_bands(mask)?;
            }
            let index = match rat {
                "LTE" => 2,
                "NR_NSA" => 4,
                "NR" => 6,
                _ => return Err(invalid()),
            };
            let mut low = 0u64;
            let mut high = 0u64;
            if rat == "LTE" {
                if !hex_bands(&fields[3])?.is_empty() {
                    return Err(PanelError::new("LOCK_VARIANT_UNSUPPORTED", 422));
                }
                let mut mask = 0u128;
                for band in bands {
                    if band > 128 {
                        return Err(invalid());
                    }
                    mask |= 1u128 << (band - 1);
                }
                fields[2] = format!("{mask:X}");
                return Ok(vec![format!("AT#BND={}", fields.join(","))]);
            }
            for band in bands {
                if band <= 64 {
                    low |= 1u64 << (band - 1);
                } else if band <= 128 {
                    high |= 1u64 << (band - 65);
                } else {
                    return Err(invalid());
                }
            }
            fields[index] = format!("{low:X}");
            fields[index + 1] = format!("{high:X}");
            format!("AT#BND={}", fields.join(","))
        }
        Kind::Air724 => {
            let p = state["wire"]["fields"]
                .as_array()
                .filter(|a| a.len() == 8)
                .ok_or_else(invalid)?;
            let mut values = p
                .iter()
                .map(|v| v.as_u64().ok_or_else(invalid))
                .collect::<Result<Vec<_>>>()?;
            if values[0] > 15 {
                return Err(invalid());
            }
            if values[0] != 5 {
                return Err(PanelError::new("LOCK_REQUIRES_LTE_MODE", 422));
            }
            let mut low = 0u32;
            let mut high = 0u32;
            for band in bands {
                if band <= 32 {
                    low |= 1 << (band - 1);
                } else if band <= 64 {
                    high |= 1 << (band - 33);
                } else {
                    return Err(invalid());
                }
            }
            values[3] = high as u64;
            values[4] = low as u64;
            format!(
                "AT*BAND={}",
                values
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Kind::SimcomLte => {
            let p = state["wire"]["fields"]
                .as_array()
                .filter(|a| [3, 4].contains(&a.len()))
                .ok_or_else(invalid)?;
            let first = p[0].as_str().ok_or_else(invalid)?;
            hex_bands(first)?;
            let mut low = 0u128;
            let mut high = 0u128;
            for band in bands {
                if band <= 32 {
                    low |= 1u128 << (band - 1);
                } else if band <= 128 {
                    high |= 1u128 << (band - 33);
                } else {
                    return Err(invalid());
                }
            }
            format!("AT+CNBP={first},0x{low:016X},0x{high:X},0")
        }
        Kind::Neoway => format!(
            "AT+NWSETBAND={},{},{}",
            if rat == "LTE" { 3 } else { 6 },
            bands.len(),
            join(",")
        ),
    };
    if command.len() > 190 {
        return Err(PanelError::new("LOCK_INVALID", 400));
    }
    Ok(vec![command])
}
fn decimal(s: &str) -> Result<u64> {
    s.parse::<u64>()
        .ok()
        .filter(|_| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
        .ok_or_else(invalid)
}
fn code_list(raw: &str) -> Result<Vec<u64>> {
    if raw.is_empty() {
        return Ok(vec![]);
    }
    let mut out = BTreeSet::new();
    for part in raw.split(',') {
        let part = part.trim();
        let n = part
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0 && *n <= 6024 && part.bytes().all(|c| c.is_ascii_digit()))
            .ok_or_else(invalid)?;
        if !out.insert(n) || out.len() > 256 {
            return Err(invalid());
        }
    }
    Ok(out.into_iter().collect())
}
fn decode_code(code: u64, rat: &str) -> Option<u64> {
    match rat {
        "LTE" if (101..500).contains(&code) => Some(code - 100),
        "NR" if (5001..=6024).contains(&code) => Some(code - 5000),
        "NR" if (501..=1524).contains(&code) => Some(code - 500),
        _ => None,
    }
}
pub fn matched_transition(model: &str, before: &Value, after: &Value, op: &Value) -> bool {
    matched(model, after, op)
        && (!templates::supported(model) || templates::preserved(before, after, op))
        && (op["mode"] == "auto" && modem_reset(model)
            || before["groups"].as_object().is_some_and(|groups| {
                groups.iter().all(|(rat, group)| {
                    op["rat"] == rat.as_str()
                        || group["selectedBands"] == after["groups"][rat]["selectedBands"]
                })
            }))
}
pub fn matched(model: &str, state: &Value, op: &Value) -> bool {
    if op["mode"] == "auto" && modem_reset(model) {
        return planes(model).iter().all(|rat| {
            let g = &state["groups"][rat];
            g["availableBands"]
                .as_array()
                .filter(|a| !a.is_empty())
                .zip(g["selectedBands"].as_array())
                .is_some_and(|(all, selected)| {
                    all.len() == selected.len() && all.iter().all(|b| selected.contains(b))
                })
        });
    }
    validate(model, op).ok().is_some_and(|mut bands| {
        if op["mode"] == "auto" {
            bands = state["groups"][op["rat"].as_str().unwrap_or("")]["availableBands"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect();
        }
        state["groups"][op["rat"].as_str().unwrap_or("")]["selectedBands"]
            .as_array()
            .is_some_and(|a| a.len() == bands.len() && bands.iter().all(|b| a.contains(&json!(b))))
    })
}
fn parentheses(mut raw: &str) -> Result<Vec<&str>> {
    let mut out = vec![];
    while let Some(tail) = raw.strip_prefix('(') {
        let (part, next) = tail.split_once(')').ok_or_else(invalid)?;
        out.push(part.trim());
        raw = next.trim();
        if raw.is_empty() {
            break;
        }
        raw = raw.strip_prefix(',').ok_or_else(invalid)?.trim();
        if out.len() > 16 {
            return Err(invalid());
        }
    }
    if !raw.is_empty() {
        return Err(invalid());
    }
    Ok(out)
}
