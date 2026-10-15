//! 参考QModem合宙LTE查询格式，解析小区标识和信号测量。
use super::{band_candidates, cell, csv, n, same_identity, supplement};
use serde_json::{Value, json};

fn valid_decimal(s: &str, max: u64) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| b.is_ascii_digit())
        && s.parse::<u64>().is_ok_and(|n| n <= max)
}
pub(super) fn parse(raw: &str) -> Value {
    let (mut serving, mut neighbors) = (vec![], vec![]);
    let mut frame = vec![];
    for line in raw.lines().map(str::trim) {
        if line == "ERROR" || line.starts_with("+CME ERROR") || line.starts_with("+CMS ERROR") {
            frame.clear();
            continue;
        }
        if line != "OK" {
            if !line.is_empty() {
                frame.push(line);
            }
            continue;
        }
        for line in frame.drain(..) {
            let (primary, body, command) =
                if let Some(v) = line.strip_prefix("+CCED:LTE current cell:") {
                    (true, v, "AT+CCED=0,1")
                } else if let Some(v) = line.strip_prefix("+CCED:LTE neighbor cell:") {
                    (false, v, "AT+CCED=0,2")
                } else {
                    continue;
                };
            let p = csv(body);
            if p.len() != if primary { 13 } else { 9 }
                || p[0].len() != 3
                || !p[0].bytes().all(|b| b.is_ascii_digit())
                || ![2, 3].contains(&p[1].len())
                || !p[1].bytes().all(|b| b.is_ascii_digit())
            {
                continue;
            }
            let (channel, pci, cid, tac, rsrp, rsrq) = if primary {
                (6, 12, 7, 10, 8, 9)
            } else {
                (2, 8, 3, 6, 4, 5)
            };
            if !valid_decimal(&p[channel], 262143)
                || !valid_decimal(&p[pci], 503)
                || !valid_decimal(&p[cid], u64::from(u32::MAX))
                || !valid_decimal(&p[tac], 65535)
            {
                continue;
            }
            let mut row = cell(
                "LTE",
                if primary { "pcc" } else { "neighbor" },
                &p[channel],
                n(&p[pci], 0., 503.),
                if primary { &p[4] } else { "" },
                command,
                json!({"mcc":p[0],"mnc":p[1],"cellId":p[cid],"tac":p[tac],"networkMode":"4G LTE","channelType":"center","metricSources":{},"metricIntervals":{}}),
            );
            let candidates = band_candidates("LTE", row["channel"].as_f64().unwrap_or(-1.));
            if candidates.is_empty()
                || primary
                    && !candidates
                        .iter()
                        .any(|b| Some(b.as_str()) == row["band"].as_str())
            {
                continue;
            }
            supplement(&mut row, "rsrp", &p[rsrp], "LTE", command);
            supplement(&mut row, "rsrq", &p[rsrq], "LTE", command);
            if primary {
                let width = match p[5].as_str() {
                    "n6" => Some(1.4),
                    "n15" => Some(3.),
                    "n25" => Some(5.),
                    "n50" => Some(10.),
                    "n75" => Some(15.),
                    "n100" => Some(20.),
                    _ => None,
                };
                row["dlBandwidthMHz"] = json!(width);
                row["dlBandwidthRaw"] = json!(p[5]);
                if width.is_some() {
                    row["metricSources"]["dlBandwidthMHz"] = json!(command);
                }
                serving.push(row);
            } else {
                neighbors.push(row);
            }
        }
    }
    if serving.len() > 1 {
        serving.clear();
    }
    neighbors.retain(|r| !serving.iter().any(|s| same_identity(s, r)));
    json!({"serving":serving,"carriers":serving,"neighbors":super::deduplicate(neighbors)})
}
