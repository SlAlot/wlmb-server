//! 参考QModem广和通LTE回复字段，解析服务测量、载波和邻区。
use super::{
    band_candidates, blocks, cell, complete_gt_block, csv, n, numeric, same_identity, supplement,
};
use serde_json::{Value, json};

fn decimal(raw: &str, max: u64) -> Option<u64> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse::<u64>().ok().filter(|v| *v <= max)
}
pub(super) fn parse(raw: &str) -> Value {
    let (mut serving, mut neighbors) = (vec![], vec![]);
    for block in blocks(raw, "+GTCCINFO:") {
        if !complete_gt_block(block) {
            continue;
        }
        for line in block.lines() {
            let p = csv(line);
            let primary = p.first().is_some_and(|s| s == "1");
            if p.len() != if primary { 14 } else { 12 }
                || !["1", "2"].contains(&p[0].as_str())
                || p[1] != "4"
                || p[2].len() != 3
                || !p[2].bytes().all(|b| b.is_ascii_digit())
                || ![2, 3].contains(&p[3].len())
                || !p[3].bytes().all(|b| b.is_ascii_digit())
                || decimal(&p[4], 65535).is_none()
                || decimal(&p[5], u64::from(u32::MAX)).is_none()
                || decimal(&p[6], 262143).is_none()
                || decimal(&p[7], 503).is_none()
            {
                continue;
            }
            let band = if primary {
                p[8].strip_prefix("BAND_LTE_").unwrap_or(&p[8])
            } else {
                ""
            };
            let mut row = cell(
                "LTE",
                if primary { "pcc" } else { "neighbor" },
                &p[6],
                n(&p[7], 0., 503.),
                band,
                "AT+GTCCINFO?",
                json!({"mcc":p[2],"mnc":p[3],"tac":p[4],"cellId":p[5],"channelType":"center","networkMode":"4G LTE","metricSources":{},"metricIntervals":{}}),
            );
            let candidates = band_candidates("LTE", row["channel"].as_f64().unwrap_or(-1.));
            if candidates.is_empty()
                || primary
                    && !candidates
                        .iter()
                        .any(|s| Some(s.as_str()) == row["band"].as_str())
            {
                continue;
            }
            supplement(
                &mut row,
                "rsrp",
                &p[if primary { 12 } else { 10 }],
                "LTE",
                "AT+GTCCINFO?",
            );
            supplement(
                &mut row,
                "rsrq",
                &p[if primary { 13 } else { 11 }],
                "LTE",
                "AT+GTCCINFO?",
            );
            if primary {
                if let Some(index) = numeric(&p[10], -99., 99.).filter(|v| v.fract() == 0.) {
                    let upper = index / 2.;
                    if (-23. ..=50.).contains(&upper) {
                        row["sinr"] = json!(upper);
                        row["metricSources"]["sinr"] = json!("AT+GTCCINFO?");
                        row["metricIntervals"]["sinr"] = json!({"lowerExclusive":upper-0.5,"upperInclusive":upper,"encoding":"quantized-index"});
                    }
                }
                row["dlBandwidthRaw"] = json!(p[9]);
                row["bandwidthEncoding"] = json!("unverified-firmware-enum");
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
