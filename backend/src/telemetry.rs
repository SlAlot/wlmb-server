//! 系统计数与真实采样差值；首次、回绕、接口变更或过期时不生成伪造速率。
use serde_json::{Value, json};
pub fn sample_delay(
    elapsed: std::time::Duration,
    period: std::time::Duration,
) -> std::time::Duration {
    period
        .saturating_sub(elapsed)
        .max(std::time::Duration::from_millis(100))
}
pub fn cpu_times(raw: &str) -> Option<(u64, u64)> {
    let mut fields = raw.lines().next()?.split_whitespace();
    if fields.next() != Some("cpu") {
        return None;
    }
    let values = fields
        .take(8)
        .map(|v| v.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    if values.len() < 4 {
        return None;
    }
    let total = values.iter().try_fold(0u64, |a, b| a.checked_add(*b))?;
    Some((total, values[3].checked_add(*values.get(4).unwrap_or(&0))?))
}
pub fn cpu_percent(previous: Option<(u64, u64)>, current: (u64, u64)) -> Option<f64> {
    let (old, old_idle) = previous?;
    let total = current.0.checked_sub(old)?;
    let idle = current.1.checked_sub(old_idle)?;
    if total == 0 || idle > total {
        return None;
    }
    Some((1000. * (total - idle) as f64 / total as f64).round_ties_even() / 10.)
}
pub fn memory(raw: &str) -> Option<Value> {
    let mut fields = std::collections::HashMap::new();
    for line in raw.lines() {
        let p = line.split_whitespace().collect::<Vec<_>>();
        if p.len() != 3 || p[2] != "kB" {
            continue;
        }
        let Some(key) = p[0].strip_suffix(':') else {
            continue;
        };
        if !key
            .chars()
            .all(|c| c.is_ascii_alphabetic() || ['_', '(', ')'].contains(&c))
        {
            continue;
        }
        if let Ok(value) = p[1].parse::<u64>() {
            fields.insert(key, value.checked_mul(1024)?);
        }
    }
    let total = *fields.get("MemTotal")?;
    if total == 0 {
        return None;
    }
    let available = if let Some(value) = fields.get("MemAvailable") {
        *value
    } else {
        ["MemFree", "Buffers", "Cached", "SReclaimable"]
            .iter()
            .try_fold(0u64, |sum, k| sum.checked_add(*fields.get(k).unwrap_or(&0)))?
            .saturating_sub(*fields.get("Shmem").unwrap_or(&0))
    }
    .min(total);
    let used = total - available;
    Some(
        json!({"totalBytes":total,"availableBytes":available,"usedBytes":used,"usedPercent":(1000.*used as f64/total as f64).round_ties_even()/10.,"source":"proc-meminfo"}),
    )
}
pub struct NetSample<'a> {
    pub interface: &'a str,
    pub mono_seconds: f64,
    pub rx: u64,
    pub tx: u64,
}
pub fn rate(previous: Option<&NetSample<'_>>, current: &NetSample<'_>) -> Option<Value> {
    let prev = previous?;
    if current.interface != prev.interface {
        return None;
    }
    let elapsed = current.mono_seconds - prev.mono_seconds;
    if !elapsed.is_finite() || !(0.2..=30.).contains(&elapsed) {
        return None;
    }
    let down = current.rx.checked_sub(prev.rx)? as f64 / elapsed;
    let up = current.tx.checked_sub(prev.tx)? as f64 / elapsed;
    Some(json!({"downloadBytesPerSecond":down,"uploadBytesPerSecond":up}))
}
pub fn station_rates(prev: (f64, u64, u64), current: (f64, u64, u64)) -> Option<Value> {
    let elapsed = current.0 - prev.0;
    if !elapsed.is_finite() || !(0.2..=40.).contains(&elapsed) {
        return None;
    }
    Some(
        json!({"uploadBytesPerSecond":current.1.checked_sub(prev.1)? as f64/elapsed,"downloadBytesPerSecond":current.2.checked_sub(prev.2)? as f64/elapsed}),
    )
}
fn interface_name(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 32
        && v.bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphanumeric() || b == b'_' || i > 0 && b".:-".contains(&b))
}
pub fn wan_device(dump: &Value) -> Option<String> {
    let mut candidates = vec![];
    for row in dump["interface"].as_array()? {
        let Some(device) = row["l3_device"].as_str() else {
            continue;
        };
        if row["up"] != true || row["interface"] == "lan" || !interface_name(device) {
            continue;
        }
        for route in row["route"].as_array().into_iter().flatten() {
            if route["target"] != "0.0.0.0"
                || route["mask"].as_u64() != Some(0)
                || route
                    .get("table")
                    .is_some_and(|v| !matches!(v.as_u64(), Some(0 | 254)))
            {
                continue;
            }
            if let Some(metric) = route
                .get("metric")
                .or_else(|| row.get("metric"))
                .unwrap_or(&json!(0))
                .as_u64()
            {
                candidates.push((0, metric, device));
            }
        }
        for route in row["route"].as_array().into_iter().flatten() {
            if !["::", "0:0:0:0:0:0:0:0"]
                .iter()
                .any(|v| route["target"] == *v)
                || route["mask"].as_u64() != Some(0)
                || route
                    .get("table")
                    .is_some_and(|v| !matches!(v.as_u64(), Some(0 | 254)))
            {
                continue;
            }
            if let Some(metric) = route
                .get("metric")
                .or_else(|| row.get("metric"))
                .unwrap_or(&json!(0))
                .as_u64()
            {
                candidates.push((1, metric, device));
            }
        }
    }
    let mut devices = std::collections::HashSet::new();
    for family in [0, 1] {
        let Some(best) = candidates
            .iter()
            .filter(|(f, _, _)| *f == family)
            .map(|(_, m, _)| *m)
            .min()
        else {
            continue;
        };
        devices.extend(
            candidates
                .iter()
                .filter(|(f, m, _)| *f == family && *m == best)
                .map(|(_, _, d)| *d),
        );
    }
    if devices.len() == 1 {
        devices.iter().next().map(|device| (*device).to_owned())
    } else {
        None
    }
}
