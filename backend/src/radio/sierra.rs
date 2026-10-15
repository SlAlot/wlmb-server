//! 参考QModem的Sierra状态与载波字段，保持服务测量和聚合库存分离。
use super::{PLMN, cell, complete_inventory, deduplicate, n, numeric};
use serde_json::{Value, json};

fn field<'a>(block: &'a str, label: &str) -> &'a str {
    block
        .find(label)
        .and_then(|at| block[at + label.len()..].split_whitespace().next())
        .unwrap_or("")
}
fn lte_bw(value: &str) -> Value {
    value
        .parse::<usize>()
        .ok()
        .and_then(|i| [1.4, 3., 5., 10., 15., 20.].get(i).copied())
        .map_or(Value::Null, |v| json!(v))
}
fn rsrp(value: &str) -> Value {
    numeric(value, -156., -28.)
        .or_else(|| numeric(value, -1560., -280.).map(|v| v / 10.))
        .map_or(Value::Null, |v| json!(v))
}
pub(super) fn parse(raw: &str, em92: bool) -> Value {
    let mut serving = vec![];
    let mut carriers = vec![];
    let mut neighbors = vec![];
    let mut section = "";
    let mut main_channel = String::new();
    let mut main_band = String::new();
    for line in raw.lines().map(str::trim) {
        if line.starts_with("!LTEINFO:") {
            section = "";
            main_channel.clear();
            main_band.clear();
            continue;
        }
        for (prefix, name) in [
            ("Serving:", "main"),
            ("IntraFreq:", "intra"),
            ("InterFreq:", "inter"),
            ("CA SCell:", "ca"),
            ("WCDMA:", ""),
        ] {
            if line.starts_with(prefix) {
                section = name;
            }
        }
        if line == "OK" || line == "ERROR" || line.starts_with("!") || line.starts_with("AT") {
            section = "";
            continue;
        }
        let p: Vec<_> = line.split_whitespace().collect();
        if p.first()
            .is_none_or(|s| !s.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let row = match section {
            "main" if p.len() == 14 => {
                if numeric(p[0], 0., 262143.).is_none() || numeric(p[9], 0., 503.).is_none() {
                    continue;
                }
                main_channel = p[0].into();
                main_band = p[5].into();
                let row = cell(
                    "LTE",
                    "pcc",
                    p[0],
                    n(p[9], 0., 503.),
                    p[5],
                    "AT!LTEINFO?",
                    json!({"mcc":p[1],"mnc":p[2],"tac":p[3],"cellId":p[4],"dlBandwidthMHz":lte_bw(p[6]),"ulBandwidthMHz":lte_bw(p[7]),"rssnr":n(p[8],-10.,30.),"rsrq":p[10],"rsrp":rsrp(p[11]),"rssi":p[12],"networkMode":"4G LTE","channelType":"center"}),
                );
                serving.push(row.clone());
                Some(row)
            }
            "ca" if p.len() == 12 => {
                if !["0", "1", "2"].contains(&p[3]) {
                    continue;
                }
                Some(cell(
                    "LTE",
                    "scc",
                    p[0],
                    n(p[8], 0., 503.),
                    p[2],
                    "AT!LTEINFO?",
                    json!({"carrierId":n(p[1],0.,503.),"active":p[3]=="2","dlBandwidthMHz":lte_bw(p[4]),"ulBandwidthMHz":lte_bw(p[5]),"rsrp":rsrp(p[9]),"rssi":p[10],"sinr":numeric(p[11],0.,250.).filter(|v|v.fract()==0.).map(|v|v/5.-20.),"channelType":"center"}),
                ))
            }
            "intra" if p.len() == 5 && !main_channel.is_empty() => {
                neighbors.push(cell(
                    "LTE",
                    "neighbor",
                    &main_channel,
                    n(p[0], 0., 503.),
                    &main_band,
                    "AT!LTEINFO?",
                    json!({"rsrq":p[1],"rsrp":rsrp(p[2]),"rssi":p[3],"channelType":"center"}),
                ));
                None
            }
            "inter" if p.len() == 9 => {
                neighbors.push(cell(
                    "LTE",
                    "neighbor",
                    p[0],
                    n(p[4], 0., 503.),
                    "",
                    "AT!LTEINFO?",
                    json!({"rsrq":p[5],"rsrp":rsrp(p[6]),"rssi":p[7],"channelType":"center"}),
                ));
                None
            }
            _ => None,
        };
        if let Some(row) = row {
            carriers.push(row);
        }
    }
    let nr = raw
        .split("!NRINFO:")
        .nth(1)
        .unwrap_or("")
        .split("\nOK")
        .next()
        .unwrap_or("");
    let mode = field(nr, "Connectivity Mode:");
    let mut nr_rows = vec![];
    if ["SA", "NSA"].contains(&mode) {
        for block in nr.split("NR5G Cell ID:").skip(1).take(32) {
            let channel = field(block, "NR5G Rx chan:");
            let band = field(block, "NR5G band:");
            if !numeric(channel, 1., 3279165.).is_some_and(|v| v.fract() == 0.)
                || !numeric(band.trim_start_matches('n'), 1., 1024.)
                    .is_some_and(|v| v.fract() == 0.)
            {
                continue;
            }
            let id = block.split_whitespace().next().unwrap_or("");
            let mut row = cell(
                "NR",
                "unknown",
                channel,
                if em92 { Value::Null } else { n(id, 0., 1007.) },
                band,
                "AT!NRINFO?",
                json!({"carrierId":n(field(block,"NR5G Carrier ID:"),0.,4294967295.),"dlBandwidthMHz":n(field(block,"NR5G dl bw:"),0.1,400.),"ulBandwidthMHz":n(field(block,"NR5G ul bw:"),0.1,400.),"rssi":field(block,"NR5G(sub6) RxM RSSI (dBm):"),"rssiSource":"RxM branch","networkMode":format!("5G {mode}"),"channelType":"center"}),
            );
            if em92 && id.len() <= 16 && !id.is_empty() && id.bytes().all(|b| b.is_ascii_hexdigit())
            {
                row["cellId"] = json!(id);
            }
            if let Some(c) = PLMN.captures(field(block, "NR5G MCC-MNC:")) {
                row["mcc"] = json!(&c[1]);
                row["mnc"] = json!(&c[2]);
            }
            nr_rows.push(row);
        }
    }
    nr_rows = deduplicate(nr_rows);
    if !nr_rows.is_empty() {
        if mode == "SA" {
            serving.clear();
            carriers.retain(|r| r["rat"] != "LTE");
        }
        let mut summary = if nr_rows.len() == 1 {
            nr_rows[0].clone()
        } else {
            cell(
                "NR",
                "unknown",
                "",
                Value::Null,
                "",
                "AT!NRINFO?",
                json!({"networkMode":format!("5G {mode}"),"measurementScope":"unbound-NR-summary"}),
            )
        };
        summary["rsrp"] = rsrp(field(nr, "NR5G RSRP (dBm):"));
        summary["rsrq"] = n(field(nr, "NR5G RSRQ (dB):"), -44., 20.);
        summary["sinr"] = n(field(nr, "NR5G SINR (dB):"), -23., 50.);
        if nr_rows.len() == 1 {
            summary["role"] = json!("pcc");
            summary["roleKnown"] = json!(true);
            summary["roleSource"] = json!("single-reported-NR-carrier");
            nr_rows[0] = summary.clone();
        }
        if mode == "NSA" {
            for row in &mut serving {
                row["networkMode"] = json!("5G NSA");
            }
        }
        serving.push(summary);
        carriers.extend(nr_rows);
    }
    neighbors.retain(|row| !serving.iter().any(|s| super::same_identity(s, row)));
    json!({"serving":serving,"carriers":complete_inventory(&[],carriers),"neighbors":deduplicate(neighbors)})
}
