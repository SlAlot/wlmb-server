//! 参考QModem的Sierra AT!BAND模板格式；不套用到上游已禁用锁频的华为平台。
use crate::{PanelError, Result, catalog};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};
fn invalid() -> PanelError {
    PanelError::new("LOCK_UNVERIFIED", 422)
}
pub(super) fn supported(model: &str) -> bool {
    let key = catalog::normalize(model);
    ["em9190", "em9291", "mc7354", "mc7355"].contains(&key.as_str())
}
fn group_name(id: &str, name: &str) -> Option<&'static str> {
    match (id, name) {
        ("0", "GW") => Some("GW"),
        ("1", "LTE") => Some("LTE"),
        ("3", "NRNSA") => Some("NR_NSA"),
        ("4", "NRSA") => Some("NR"),
        _ => None,
    }
}
fn mask(text: &str) -> Result<u64> {
    u64::from_str_radix(text, 16).map_err(|_| invalid())
}
fn bands(low: u64, high: u64) -> Vec<u64> {
    (1..=128)
        .filter(|n| {
            if *n <= 64 {
                low & (1u64 << (*n - 1)) != 0
            } else {
                high & (1u64 << (*n - 65)) != 0
            }
        })
        .collect()
}
pub(super) fn parse(model: &str, replies: &BTreeMap<String, String>) -> Result<Value> {
    static ACTIVE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r"^([0134])\s*-\s*(GW|LTE|NRNSA|NRSA):\s*([0-9A-Fa-f]{1,16})(?:[\s,]+([0-9A-Fa-f]{1,16}))?$").ok()
    });
    static CLASS: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"^([0134])\s*-\s*(GW|LTE|NRNSA|NRSA):$").ok());
    static ENTRY: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r"^[0-9A-Fa-f]{1,32}\s*-\s*([BbNn])([0-9]{1,3})(?:\s+[^\r\n]*)?$").ok()
    });
    let active = ACTIVE.as_ref().ok_or_else(invalid)?;
    let class = CLASS.as_ref().ok_or_else(invalid)?;
    let entry = ENTRY.as_ref().ok_or_else(invalid)?;
    let current = replies.get("AT!BAND?").ok_or_else(invalid)?;
    let list = replies.get("AT!BAND=?").ok_or_else(invalid)?;
    let mut masks: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for line in super::payload(current, "AT!BAND?")? {
        let line = line.strip_prefix("!BAND:").unwrap_or(line).trim();
        if let Some(c) = active.captures(line) {
            let name = group_name(&c[1], &c[2]).ok_or_else(invalid)?;
            let pair = (
                mask(&c[3])?,
                c.get(4).map(|m| mask(m.as_str())).transpose()?.unwrap_or(0),
            );
            if masks.insert(name.into(), pair).is_some() {
                return Err(invalid());
            }
        } else if line.contains(" - ")
            || line.starts_with("0 -")
            || line.starts_with("1 -")
            || line.starts_with("3 -")
            || line.starts_with("4 -")
        {
            return Err(invalid());
        }
    }
    // 激活自定义模板会作用整个配置，旧GW值缺失时不能猜测，更不能套用其他模板的默认值。
    if !masks.contains_key("GW") || masks["GW"].1 != 0 {
        return Err(invalid());
    }
    let mut available: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
    let mut started = false;
    let mut current_class = None;
    for line in super::payload(list, "AT!BAND=?")? {
        let line = line.trim();
        if line == "Available:" {
            if started {
                return Err(invalid());
            }
            started = true;
            continue;
        }
        if !started {
            continue;
        }
        if let Some(c) = class.captures(line) {
            let name = group_name(&c[1], &c[2]).ok_or_else(invalid)?;
            if available.insert(name.into(), BTreeSet::new()).is_some() {
                return Err(invalid());
            }
            current_class = Some(name);
            continue;
        }
        let group = current_class.ok_or_else(invalid)?;
        if group == "GW" {
            continue;
        }
        let captures = entry.captures(line).ok_or_else(invalid)?;
        let band = captures[2].parse::<u64>().map_err(|_| invalid())?;
        if !(1..=128).contains(&band)
            || group == "LTE" && !captures[1].eq_ignore_ascii_case("B")
            || group != "LTE" && !captures[1].eq_ignore_ascii_case("N")
        {
            return Err(invalid());
        }
        if !available.get_mut(group).ok_or_else(invalid)?.insert(band) {
            return Err(invalid());
        }
    }
    if !started {
        return Err(invalid());
    }
    let mut groups = serde_json::Map::new();
    let mut wire = serde_json::Map::new();
    for (rat, (low, high)) in masks {
        wire.insert(
            rat.clone(),
            json!([format!("{low:016X}"), format!("{high:016X}")]),
        );
        if rat == "GW" {
            continue;
        }
        let all = available.get(&rat).ok_or_else(invalid)?;
        let selected = bands(low, high);
        if selected.iter().any(|b| !all.contains(b)) {
            return Err(invalid());
        }
        if all.is_empty() && selected.is_empty() {
            continue;
        }
        if !super::planes(model).contains(&rat.as_str()) {
            return Err(invalid());
        }
        groups.insert(rat.clone(),json!({"rat":rat,"mode":if all.len()==selected.len(){0}else{3},"entries":selected.iter().map(|b|json!({"band":b})).collect::<Vec<_>>(),"selectedBands":selected,"availableBands":all,"availableSource":"firmware","target":"band"}));
    }
    if groups.is_empty() {
        return Err(invalid());
    }
    Ok(json!({"groups":groups,"wire":{"templateMasks":wire}}))
}
pub(super) fn commands(op: &Value, state: &Value, selected: &[u64]) -> Result<Vec<String>> {
    let target = op["rat"].as_str().ok_or_else(invalid)?;
    let masks = state["wire"]["templateMasks"]
        .as_object()
        .ok_or_else(invalid)?;
    if !masks.contains_key("GW") || !masks.contains_key(target) {
        return Err(invalid());
    }
    let mut commands = vec![];
    for (rat, id) in [("GW", 0), ("LTE", 1), ("NR_NSA", 3), ("NR", 4)] {
        let Some(pair) = masks.get(rat) else { continue };
        let fields = pair
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or_else(invalid)?;
        let mut low = mask(fields[0].as_str().ok_or_else(invalid)?)?;
        let mut high = mask(fields[1].as_str().ok_or_else(invalid)?)?;
        if rat == target {
            low = 0;
            high = 0;
            for band in selected {
                if *band <= 64 {
                    low |= 1u64 << (band - 1);
                } else if *band <= 128 {
                    high |= 1u64 << (band - 65);
                } else {
                    return Err(invalid());
                }
            }
        }
        if rat == "GW" {
            if high != 0 {
                return Err(invalid());
            }
            commands.push(format!("AT!BAND=0F,1,\"Custom\",{id},{low:016X}"));
        } else {
            commands.push(format!(
                "AT!BAND=0F,1,\"Custom\",{id},{low:016X},{high:016X}"
            ));
        }
    }
    commands.push("AT!BAND=0F".into());
    Ok(commands)
}
pub(super) fn preserved(before: &Value, after: &Value, op: &Value) -> bool {
    before["wire"]["templateMasks"]
        .as_object()
        .zip(after["wire"]["templateMasks"].as_object())
        .is_some_and(|(a, b)| {
            a.contains_key("GW")
                && a.len() == b.len()
                && a.iter().all(|(rat, mask)| {
                    b.contains_key(rat) && (op["rat"] == rat.as_str() || b[rat] == *mask)
                })
        })
}
