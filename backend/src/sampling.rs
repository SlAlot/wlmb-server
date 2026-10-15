//! 采集结果归并与字段有效期；忙碌保留有效快照，过期和身份变更失效。
use crate::{MAX_AT_RESPONSE, PanelError, Result, radio};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Instant};
#[derive(Clone)]
pub struct Sample {
    pub raw: String,
    pub wall_ms: u64,
    pub mono: Instant,
}
pub fn primary_query<'a>(
    modem: &Value,
    commands: &'a [String],
    samples: &BTreeMap<String, Sample>,
    now: u64,
) -> Option<&'a str> {
    let parser = modem["profile"]["parser"].as_str().unwrap_or("generic");
    if matches!(parser, "sierra-em91" | "sierra-em92") {
        return commands
            .iter()
            .take(2)
            .find(|command| {
                samples.get(*command).is_some_and(|sample| {
                    sample.wall_ms <= now
                        && now - sample.wall_ms < 30000
                        && radio::parse(parser, &sample.raw).ok().is_some_and(|value| {
                            value["serving"]
                                .as_array()
                                .is_some_and(|rows| !rows.is_empty())
                        })
                })
            })
            .map(String::as_str);
    }
    commands.first().map(String::as_str)
}
pub fn age_temperature(snapshot: &mut Value, now: u64) {
    if let Some(sensor) = snapshot.get_mut("temperature").filter(|v| v.is_object()) {
        sensor["stale"] = json!(
            sensor["stale"] == true
                || sensor["observedAtMs"]
                    .as_u64()
                    .is_none_or(|at| now < at || now - at > 90000)
        );
    }
}
pub fn age_metrics(row: &mut Value, now: u64) {
    if let Some(stamps) = row
        .get("metricObservedAtMs")
        .and_then(Value::as_object)
        .cloned()
    {
        for (metric, stamp) in stamps {
            if stamp.as_u64().is_none_or(|at| at > now || now - at > 15000) {
                row[&metric] = Value::Null;
                if let Some(intervals) = row
                    .get_mut("metricIntervals")
                    .and_then(Value::as_object_mut)
                {
                    intervals.remove(&metric);
                }
            }
        }
    }
}
pub fn publish(
    modem: &Value,
    commands: &[String],
    samples: &BTreeMap<String, Sample>,
    errors: &[Value],
    now: u64,
) -> Result<Value> {
    if commands.is_empty()
        || commands.len() > crate::MAX_SAMPLE_QUERIES
        || samples.len() > crate::MAX_SAMPLE_QUERIES
        || samples.values().any(|s| s.raw.len() > MAX_AT_RESPONSE)
    {
        return Err(PanelError::new("AT_FAILED", 503));
    }
    let mut kept = samples
        .iter()
        .filter(|(command, sample)| {
            sample.wall_ms <= now
                && now.saturating_sub(sample.wall_ms)
                    < if crate::temperature::slow_query(command) {
                        90000
                    } else {
                        30000
                    }
        })
        .map(|(command, sample)| (command.clone(), sample))
        .collect::<BTreeMap<_, _>>();
    let primary_key = primary_query(modem, commands, samples, now);
    let primary = primary_key.and_then(|key| kept.get(key).copied());
    if modem["profile"]["parser"] == "fibocom-fm350"
        && kept
            .get("AT+CESQ")
            .is_some_and(|v| primary.is_none_or(|p| v.wall_ms < p.wall_ms))
    {
        kept.remove("AT+CESQ");
    }
    let raw = commands
        .iter()
        .filter_map(|key| kept.get(key).map(|s| s.raw.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    let parser = modem["profile"]["parser"].as_str().unwrap_or("generic");
    let mut parsed = radio::parse_snapshot(parser, &raw)?;
    let model = modem["profile"]["model"].as_str().unwrap_or("");
    if !crate::temperature::queries(model).is_empty() {
        parsed["temperature"] = crate::temperature::queries(model)
            .iter()
            .filter_map(|query| {
                let sample = kept.get(*query)?;
                let mut value = crate::temperature::response(model, query, &sample.raw)?;
                value["observedAtMs"] = json!(sample.wall_ms);
                Some((sample.wall_ms, value))
            })
            .max_by_key(|(stamp, _)| *stamp)
            .map(|(_, value)| value)
            .unwrap_or(Value::Null);
    }
    for group in ["serving", "carriers", "neighbors"] {
        if let Some(rows) = parsed[group].as_array_mut() {
            for row in rows {
                let source = row["source"].as_str().unwrap_or("").replace('"', "");
                let sample = kept
                    .iter()
                    .find(|(command, _)| command.replace('"', "") == source)
                    .map(|(_, s)| *s)
                    .or(primary);
                row["observedAtMs"] = json!(sample.map(|s| s.wall_ms));
                row["stale"] = json!(sample.is_none_or(|s| now.saturating_sub(s.wall_ms) > 15000));
                if let Some(sources) = row
                    .get("metricSources")
                    .and_then(Value::as_object)
                    .filter(|m| !m.is_empty())
                    .cloned()
                {
                    row["metricObservedAtMs"] = json!({});
                    for (metric, source) in sources {
                        row["metricObservedAtMs"][metric] =
                            json!(source.as_str().and_then(|s| kept.get(s).map(|s| s.wall_ms)));
                    }
                    age_metrics(row, now);
                }
            }
        }
    }
    if let Some(row) = parsed
        .get_mut("secondaryMeasurement")
        .filter(|row| row.is_object())
    {
        let sample = row["source"].as_str().and_then(|source| kept.get(source));
        row["observedAtMs"] = json!(sample.map(|sample| sample.wall_ms));
        row["stale"] =
            json!(sample.is_none_or(|sample| now.saturating_sub(sample.wall_ms) > 15000));
    }
    let primary_errors: Vec<_> = errors
        .iter()
        .filter(|e| primary_key.is_some_and(|cmd| e["command"] == cmd))
        .collect();
    let deferred = !primary_errors.is_empty()
        && primary_errors.iter().all(|e| {
            matches!(
                e["code"].as_str(),
                Some(
                    "AT_BUSY"
                        | "SYSTEM_BUSY"
                        | "AT_COOLDOWN"
                        | "AT_PORT_CONFLICT"
                        | "AT_OWNERSHIP_UNKNOWN"
                )
            )
        });
    let primary_failed = !primary_errors.is_empty() && !deferred;
    let primary_stale = primary.is_none_or(|s| now < s.wall_ms || now - s.wall_ms > 15000);
    parsed["modem"] = modem.clone();
    parsed["observations"] = json!(
        kept.iter()
            .map(|(key, s)| json!({"command":key,"observedAtMs":s.wall_ms}))
            .collect::<Vec<_>>()
    );
    parsed["errors"] = json!(errors);
    parsed["observedAtMs"] = json!(primary.map_or(now, |s| s.wall_ms));
    parsed["partial"] = json!(!errors.is_empty());
    parsed["pending"] = json!(false);
    // 通道短暂忙碌不撤销仍有效的测量；原时间戳不前移，过期和身份错误仍失效。
    parsed["stale"] = json!(primary_failed || primary_stale);
    parsed["refreshDeferred"] = json!(deferred && !primary_stale);
    for key in [
        "subscription",
        "sessionQos",
        "smsReceiving",
        "networkOperator",
    ] {
        if let Some(item) = parsed.get_mut(key) {
            let sample = item["source"].as_str().and_then(|s| {
                kept.get(s).or_else(|| {
                    (key == "networkOperator")
                        .then(|| {
                            kept.iter()
                                .find(|(command, _)| command.replace('"', "") == s.replace('"', ""))
                                .map(|(_, sample)| sample)
                        })
                        .flatten()
                })
            });
            item["observedAtMs"] = json!(sample.map(|s| s.wall_ms));
            let ttl = if key == "networkOperator" && item["source"] != "AT+COPS?" {
                15000
            } else {
                90000
            };
            item["stale"] = json!(sample.is_none_or(|s| now.saturating_sub(s.wall_ms) > ttl));
        }
    }
    if parsed["networkOperator"]["stale"] == true {
        parsed["operator"] = Value::Null;
    }
    age_temperature(&mut parsed, now);
    Ok(parsed)
}
