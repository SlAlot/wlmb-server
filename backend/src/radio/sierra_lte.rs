//! 参考QModem的Sierra LTE状态字段，校验信号单位和驻留标识。
use super::{cell, n};
use serde_json::{Value, json};

fn field<'a>(raw: &'a str, label: &str) -> Option<&'a str> {
    let mut hits = raw.match_indices(label);
    let (at, _) = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    raw[at + label.len()..].split_whitespace().next()
}
pub(super) fn parse(raw: &str) -> Value {
    let empty = || json!({"serving":[],"carriers":[],"neighbors":[]});
    if raw.matches("!GSTATUS:").count() != 1
        || !raw.lines().last().is_some_and(|s| s.trim() == "OK")
        || raw
            .lines()
            .any(|s| s.trim() == "ERROR" || s.trim().starts_with("+CME ERROR"))
        || field(raw, "System mode:") != Some("LTE")
        || field(raw, "Mode:") != Some("ONLINE")
        || field(raw, "PS state:") != Some("Attached")
    {
        return empty();
    }
    let band = field(raw, "LTE band:")
        .unwrap_or("")
        .trim_start_matches('B');
    let channel = field(raw, "LTE Rx chan:").unwrap_or("");
    let mut row = cell(
        "LTE",
        "pcc",
        channel,
        Value::Null,
        band,
        "AT!GSTATUS?",
        json!({
            "networkMode":"4G LTE","channelType":"center",
            "cellId":field(raw,"Cell ID:"),"tac":field(raw,"TAC:"),
            "rsrp":field(raw,"RSRP (dBm):"),"rsrq":field(raw,"RSRQ (dB):"),
            "rssi":field(raw,"RSSI (dBm):"),"sinr":field(raw,"SINR (dB):"),
            "dlBandwidthMHz":field(raw,"LTE bw:").map(|s|n(s,1.4,20.)).filter(|v|v.as_f64().is_some_and(|b|[1.4,3.,5.,10.,15.,20.].contains(&b))),
            "measurementScope":"serving-cell-summary"
        }),
    );
    let consistent = row["channel"].as_f64().is_some_and(|c| {
        super::band_candidates("LTE", c)
            .iter()
            .any(|b| Some(b.as_str()) == row["band"].as_str())
    });
    if !consistent {
        return empty();
    }
    row["roleKnown"] = json!(true);
    json!({"serving":[row.clone()],"carriers":[row],"neighbors":[]})
}
