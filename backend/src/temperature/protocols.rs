//! 参考FUjr/QModem公开温度格式，按厂商与平台处理单位、传感器及备用查询。
use crate::{catalog, radio};
use serde_json::{Value, json};

pub(super) fn queries(model: &str) -> &'static [&'static str] {
    let key = catalog::normalize(model);
    let family = catalog::MODELS
        .iter()
        .find(|(_, models)| models.split_whitespace().any(|name| name == key))
        .map(|(family, _)| *family);
    match (family, catalog::platform(&key)) {
        (Some("tdtech"), _) => &["AT^CHIPTEMP?"],
        (Some("quectel"), _) => &["AT+QTEMP"],
        (Some("fibocom"), "mediatek") => &["AT+GTSENRDTEMP=1"],
        (Some("fibocom"), "unisoc") => &["AT+MTSM=1"],
        (Some("fibocom"), "qualcomm") => &["AT+MTSM=1,6", "AT+GTLADC"],
        (Some("fibocom"), _) => &["AT+MTSM=1,6", "AT+MTSM=1"],
        (Some("meig"), _) => &["AT+TEMP"],
        (Some("sierra"), "lte") => &["AT!GSTATUS?"],
        (Some("sierra"), _) => &["AT!PCTEMP?"],
        (Some("simcom"), _) => &["AT+CPMUTEMP"],
        (Some("telit"), _) => &["AT#TEMPSENS=2"],
        (Some("gosuncn"), _) => &["AT+MTSM=1"],
        (Some("foxconn"), _) if ["t99w368", "t99w640"].contains(&key.as_str()) => &["AT+TEMP?"],
        (Some("foxconn" | "thales"), _) => &["AT^TEMP?"],
        _ => &[],
    }
}
pub(super) fn slow_query(command: &str) -> bool {
    [
        "AT^CHIPTEMP?",
        "AT+QTEMP",
        "AT+GTSENRDTEMP=1",
        "AT+MTSM=1",
        "AT+MTSM=1,6",
        "AT+GTLADC",
        "AT+TEMP",
        "AT!PCTEMP?",
        "AT+CPMUTEMP",
        "AT#TEMPSENS=2",
        "AT+TEMP?",
        "AT^TEMP?",
    ]
    .contains(&command)
}
fn result(source: &str, sensor: &str, celsius: f64) -> Value {
    json!({"celsius":celsius,"source":source,"sensors":[{"sensor":sensor,"celsius":celsius}],"selection":"reported-module-sensor"})
}
fn value(raw: &str, scale: f64) -> Option<f64> {
    let n = radio::numeric(raw.trim().trim_matches('"'), -40000., 120000.)?;
    if [-273., -32768., 32767., 65535.].contains(&n) {
        return None;
    }
    let c = n / scale;
    (-40. ..=120.).contains(&c).then_some(c)
}
fn single<'a>(raw: &'a str, prefix: &str) -> Option<&'a str> {
    let mut rows = raw
        .lines()
        .filter_map(|line| line.trim().strip_prefix(prefix));
    let row = rows.next()?;
    rows.next().is_none().then_some(row.trim())
}
fn sensor_records(raw: &str, prefix: &str, sensor: &str, scale: f64) -> Option<f64> {
    let rows = radio::records(raw, prefix);
    if rows.len() > 32 {
        return None;
    }
    let mut hits = rows
        .iter()
        .filter(|row| row.first().is_some_and(|name| name == sensor));
    let row = hits.next()?;
    if hits.next().is_some() || row.len() != 2 {
        return None;
    }
    value(&row[1], scale)
}
fn chip(model: &str, raw: &str) -> Option<Value> {
    if catalog::normalize(model) == "mt5700m-cn" {
        return super::mt5700(raw);
    }
    let parts: Vec<_> = single(raw, "^CHIPTEMP:")?.split(',').collect();
    if !(6..=16).contains(&parts.len()) {
        return None;
    }
    let scale = if catalog::platform(model) == "unisoc" {
        1.
    } else {
        10.
    };
    Some(result(
        "AT^CHIPTEMP?",
        "module-chip",
        value(parts[5], scale)?,
    ))
}
fn meig(model: &str, raw: &str) -> Option<Value> {
    let (sensor, scale) = if catalog::platform(model) == "unisoc" {
        ("soc-thmzone", 1000.)
    } else {
        ("cpu0-0-usr", 1.)
    };
    Some(result(
        "AT+TEMP",
        sensor,
        sensor_records(raw, "TEMP:", sensor, scale)?,
    ))
}
fn tsens(command: &str, raw: &str) -> Option<Value> {
    let mut hits = raw.lines().filter_map(|line| {
        let line = line.trim();
        let line = line
            .strip_prefix("^TEMP:")
            .or_else(|| line.strip_prefix("+TEMP:"))
            .unwrap_or(line)
            .trim();
        line.strip_prefix("TSENS:").map(str::trim)
    });
    let line = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    let c = value(line.strip_suffix('C')?.trim(), 1.)?;
    Some(result(command, "TSENS", c))
}
fn scaled_sensor(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    if let Some(n) = raw.strip_suffix("mC") {
        return value(n, 1000.);
    }
    if let Some(n) = raw.strip_suffix('C') {
        return value(n, 1.);
    }
    let n = radio::numeric(raw, -40000., 120000.)?;
    // 上游这些平台有摄氏和毫摄氏返回；只接受合理摄氏值或整千量级整数，不截取前两位。
    if (-40. ..=120.).contains(&n) {
        value(raw, 1.)
    } else if n.fract() == 0. && n.abs() >= 1000. {
        value(raw, 1000.)
    } else {
        None
    }
}
fn gtladc(raw: &str) -> Option<Value> {
    let mut readings = vec![];
    for line in raw.lines() {
        let line = line
            .trim()
            .strip_prefix("+GTLADC:")
            .unwrap_or(line.trim())
            .trim();
        let line = line.trim_start_matches('"');
        if let Some(rest) = line.strip_prefix("cpu") {
            let n = rest.trim_start_matches(['"', ',', ':', ' ', '\t']);
            readings.push(scaled_sensor(n)?);
        }
    }
    if readings.len() != 1 {
        return None;
    }
    Some(result("AT+GTLADC", "cpu", readings[0]))
}
pub(super) fn parse(model: &str, command: &str, raw: &str) -> Option<Value> {
    match command {
        "AT^CHIPTEMP?" => chip(model, raw),
        "AT+QTEMP" => super::quectel(raw),
        "AT!GSTATUS?" => super::sierra(raw),
        "AT+TEMP" => meig(model, raw),
        "AT+TEMP?" | "AT^TEMP?" => tsens(command, raw),
        "AT+MTSM=1" | "AT+MTSM=1,6" => Some(result(
            command,
            "module",
            value(single(raw, "+MTSM:")?, 1.)?,
        )),
        "AT+CPMUTEMP" => Some(result(
            command,
            "module",
            value(single(raw, "+CPMUTEMP:")?, 1.)?,
        )),
        "AT+GTLADC" => gtladc(raw),
        "AT+GTSENRDTEMP=1" => {
            let rows = radio::records(raw, "+GTSENRDTEMP:");
            if rows.len() != 1
                || rows[0].len() != 2
                || !rows[0][0].bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            Some(result(command, "module", scaled_sensor(&rows[0][1])?))
        }
        "AT#TEMPSENS=2" => Some(result(
            command,
            "TSENS",
            sensor_records(raw, "#TEMPSENS:", "TSENS", 1.)?,
        )),
        "AT!PCTEMP?" => {
            let mut readings = raw.lines().filter_map(|line| {
                let line = line.trim();
                line.strip_prefix("!PCTEMP:")
                    .unwrap_or(line)
                    .trim()
                    .strip_prefix("Temperature:")
            });
            let text = readings.next()?.trim();
            let number = text
                .strip_suffix("degrees C")
                .or_else(|| text.strip_suffix('C'))
                .unwrap_or(text)
                .trim();
            if readings.next().is_some() {
                return None;
            }
            Some(result(command, "module", value(number, 1.)?))
        }
        _ => None,
    }
}
