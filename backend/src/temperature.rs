//! 参考QModem公开温度命令与平台格式，自行实现传感器选择、边界与缓存接入。
use serde_json::{Value, json};
mod protocols;
pub fn queries(model: &str) -> &'static [&'static str] {
    protocols::queries(model)
}
pub fn slow_query(command: &str) -> bool {
    crate::session_qos::slow_query(command)
        || command == "AT+COPS?"
        || protocols::slow_query(command)
}
pub fn query(model: &str) -> Option<&'static str> {
    queries(model).first().copied()
}
pub fn modem(model: &str, raw: &str) -> Option<Value> {
    queries(model)
        .iter()
        .find_map(|command| response(model, command, raw))
}
pub fn response(model: &str, command: &str, raw: &str) -> Option<Value> {
    if !queries(model).contains(&command) || !complete(raw) {
        return None;
    }
    protocols::parse(model, command, raw)
}
fn complete(raw: &str) -> bool {
    if raw.len() > crate::MAX_AT_RESPONSE || raw.contains('\0') {
        return false;
    }
    let lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    !lines.is_empty()
        && lines.len() <= 128
        && lines.last() == Some(&"OK")
        && !lines[..lines.len() - 1].iter().any(|s| {
            *s == "OK"
                || *s == "ERROR"
                || s.starts_with("+CME ERROR")
                || s.starts_with("+CMS ERROR")
        })
}
pub fn quectel(raw: &str) -> Option<Value> {
    if !complete(raw) {
        return None;
    }
    let mut readings = vec![];
    let mut chip_readings = vec![];
    let mut seen = std::collections::BTreeSet::new();
    for row in raw.lines().filter_map(|s| s.trim().strip_prefix("+QTEMP:")) {
        let Some((name, value)) = row.split_once(',') else {
            if !seen.is_empty() || !seen.insert("module") {
                return None;
            }
            let celsius = crate::radio::numeric(row.trim().trim_matches('"'), -40., 120.)?;
            readings.push(json!({"sensor":"module","celsius":celsius}));
            continue;
        };
        let name = name.trim().strip_prefix('"')?.strip_suffix('"')?;
        if name.len() > 64 || seen.contains("module") || !seen.insert(name) || seen.len() > 32 {
            return None;
        }
        let core = [
            "mdm-usr",
            "mdmss-0-usr",
            "mdmss-1-usr",
            "mdmss-2-usr",
            "mdmss-3-usr",
            "mdmq6-0-usr",
            "tsens_tz_sensor4",
        ]
        .contains(&name);
        let chip = ["soc-thermal", "soc-thmzone", "cpuss-usr", "cpu0-0-usr"].contains(&name);
        if !core && !chip {
            continue;
        }
        if let Some(value) = crate::radio::numeric(value.trim().trim_matches('"'), -40., 120.) {
            let item = json!({"sensor":name,"celsius":value});
            if core {
                readings.push(item);
            } else {
                chip_readings.push(item);
            }
        }
    }
    let selection = if readings.is_empty() {
        readings = chip_readings;
        "max-module-soc-sensor"
    } else {
        "max-modem-sensor"
    };
    let celsius = readings
        .iter()
        .filter_map(|r| r["celsius"].as_f64())
        .reduce(f64::max)?;
    Some(json!({"celsius":celsius,"source":"AT+QTEMP","sensors":readings,"selection":selection}))
}
pub fn sierra(raw: &str) -> Option<Value> {
    if !complete(raw) || raw.matches("!GSTATUS:").count() != 1 {
        return None;
    }
    let mut hits = raw.match_indices("Temperature:");
    let (at, _) = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    let celsius = crate::radio::numeric(
        raw[at + "Temperature:".len()..].split_whitespace().next()?,
        -40.,
        120.,
    )?;
    Some(
        json!({"celsius":celsius,"source":"AT!GSTATUS?","sensors":[{"sensor":"module","celsius":celsius}],"selection":"reported-module-sensor"}),
    )
}
pub fn milli_celsius(raw: &str) -> Option<f64> {
    let n = raw.trim().parse::<i64>().ok()?;
    (-40000..=150000).contains(&n).then_some(n as f64 / 1000.)
}
pub fn mt5700(raw: &str) -> Option<Value> {
    if !complete(raw) {
        return None;
    }
    let lines: Vec<_> = raw
        .lines()
        .filter_map(|s| s.trim().strip_prefix("^CHIPTEMP:"))
        .collect();
    if lines.len() != 1 {
        return None;
    }
    let parts: Vec<_> = lines[0].split(',').map(str::trim).collect();
    if parts.len() != 12 {
        return None;
    }
    let sensor = |i: usize| {
        parts[i]
            .parse::<i32>()
            .ok()
            .filter(|v| (-400..=1200).contains(v))
            .map(|v| v as f64 / 10.)
    };
    let readings: Vec<Value> = [8, 9]
        .into_iter()
        .filter_map(|i| {
            sensor(i).map(|v| json!({"sensor":if i==8 {"modem1"}else{"modem2"},"celsius":v}))
        })
        .collect();
    let celsius = readings
        .iter()
        .filter_map(|r| r["celsius"].as_f64())
        .reduce(f64::max)?;
    Some(
        json!({"celsius":celsius,"source":"AT^CHIPTEMP?","sensors":readings,"selection":"max-modem-sensor"}),
    )
}
#[cfg(unix)]
pub fn device() -> Option<Value> {
    use std::path::Path;
    let mut readings = vec![];
    for i in 0..32 {
        let dir = Path::new("/sys/class/thermal").join(format!("thermal_zone{i}"));
        let kind = crate::transport::bounded_read(&dir.join("type"), 128).ok();
        if !kind.as_deref().is_some_and(|s| {
            [
                "cpu-thermal",
                "cpu_thermal",
                "soc-thermal",
                "soc_thermal",
                "x86_pkg_temp",
                "cpu-thermal-zone",
            ]
            .contains(&s.trim())
        }) {
            continue;
        }
        if let Some(n) = crate::transport::bounded_read(&dir.join("temp"), 64)
            .ok()
            .and_then(|s| milli_celsius(&s))
        {
            readings.push(json!({"sensor":kind.unwrap_or_default().trim(),"celsius":n}));
        }
    }
    let celsius = readings
        .iter()
        .filter_map(|v| v["celsius"].as_f64())
        .reduce(f64::max)?;
    Some(json!({"celsius":celsius,"source":"sysfs-cpu-thermal","sensors":readings}))
}
