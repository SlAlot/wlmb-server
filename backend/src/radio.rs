//! 参考QModem公开采集协议，自行解析信号、载波与邻区；不伪造缺失字段。
use crate::{MAX_AT_RESPONSE, PanelError, Result};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;
mod fibocom_lte;
mod openluat;
mod sierra;
mod sierra_lte;
mod vendor_csv;
mod vendor_labels;

pub const NR_RANGES: &[(i64, i64, i64)] = &[
    (1, 422000, 434000),
    (2, 386000, 398000),
    (3, 361000, 376000),
    (5, 173800, 178800),
    (7, 524000, 538000),
    (8, 185000, 192000),
    (12, 145800, 149200),
    (13, 149200, 151200),
    (14, 151600, 153600),
    (18, 172000, 175000),
    (20, 158200, 164200),
    (25, 386000, 399000),
    (26, 171800, 178800),
    (28, 151600, 160600),
    (29, 143400, 145600),
    (30, 470000, 472000),
    (34, 402000, 405000),
    (38, 514000, 524000),
    (39, 376000, 384000),
    (40, 460000, 480000),
    (41, 499200, 537999),
    (48, 636667, 646666),
    (53, 496700, 499000),
    (66, 422000, 440000),
    (70, 399000, 404000),
    (71, 123400, 130400),
    (75, 286400, 303400),
    (76, 285400, 286400),
    (77, 620000, 680000),
    (78, 620000, 653333),
    (79, 693334, 733333),
    (257, 2054166, 2104165),
    (258, 2016667, 2070832),
    (260, 2229166, 2279165),
    (261, 2070833, 2084999),
];
pub const LTE_RANGES: &[(i64, i64, i64)] = &[
    (1, 0, 599),
    (2, 600, 1199),
    (3, 1200, 1949),
    (4, 1950, 2399),
    (5, 2400, 2649),
    (6, 2650, 2749),
    (7, 2750, 3449),
    (8, 3450, 3799),
    (9, 3800, 4149),
    (10, 4150, 4749),
    (11, 4750, 4949),
    (12, 5010, 5179),
    (13, 5180, 5279),
    (14, 5280, 5379),
    (17, 5730, 5849),
    (18, 5850, 5999),
    (19, 6000, 6149),
    (20, 6150, 6449),
    (21, 6450, 6599),
    (25, 8040, 8689),
    (26, 8690, 9039),
    (28, 9210, 9659),
    (29, 9660, 9769),
    (30, 9770, 9869),
    (31, 9870, 9919),
    (32, 9920, 10359),
    (34, 36200, 36349),
    (38, 37750, 38249),
    (39, 38250, 38649),
    (40, 38650, 39649),
    (41, 39650, 41589),
    (42, 41590, 43589),
    (43, 43590, 45589),
    (46, 46790, 54539),
    (48, 55240, 56739),
    (53, 60140, 60254),
    (66, 66436, 67335),
    (67, 67336, 67535),
    (70, 68336, 68585),
    (71, 68586, 68935),
];

fn compiled(pattern: &str) -> Regex {
    match Regex::new(pattern) {
        Ok(value) => value,
        Err(_) => panic!("invalid built-in parser regex"),
    }
}
static BAND: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?i)^(LTE|NR5G) BAND (\d+)$"));
static SIM_BAND: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"(?i)^(?:EUTRAN|NR5G)[_-]BAND(\d+)$"));
static PLMN: LazyLock<Regex> = LazyLock::new(|| compiled(r"^(\d{3})-(\d{2,3})$"));
static MHZ: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?i)^(\d+(?:\.\d+)?)M(?:Hz)?$"));
static FM_BAND: LazyLock<Regex> = LazyLock::new(|| compiled(r"^50[1-9]\d{0,2}$"));
static GT_CA: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"(?i)^\s*(PCC|SCC\s*\d+)\s*:\s*([^\r\n]+)$"));

pub fn numeric(text: &str, low: f64, high: f64) -> Option<f64> {
    let v = text.trim().parse::<f64>().ok()?;
    (v.is_finite() && v >= low && v <= high).then_some(v)
}
fn number(text: &str) -> Option<f64> {
    numeric(text, f64::MIN, f64::MAX)
}
fn n(text: &str, low: f64, high: f64) -> Value {
    numeric(text, low, high).map_or(Value::Null, |v| json!(v))
}
fn hex(text: &str) -> Value {
    i64::from_str_radix(
        text.trim()
            .trim_start_matches("0x")
            .trim_start_matches("0X"),
        16,
    )
    .map_or(Value::Null, |v| json!(v))
}
fn as_num(value: &Value) -> Option<f64> {
    value.as_f64()
}
fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn scaled(value: &str, divisor: f64) -> Value {
    number(value).map_or(Value::Null, |v| json!(v / divisor))
}

pub fn band_candidates(rat: &str, channel: f64) -> Vec<String> {
    let (prefix, table) = match rat {
        "NR" => ("n", NR_RANGES),
        "LTE" => ("B", LTE_RANGES),
        _ => return vec![],
    };
    table
        .iter()
        .filter(|(_, start, end)| channel >= *start as f64 && channel <= *end as f64)
        .map(|(b, _, _)| format!("{prefix}{b}"))
        .collect()
}
fn cell(
    rat: &str,
    role: &str,
    channel: &str,
    pci: Value,
    band: &str,
    source: &str,
    extras: Value,
) -> Value {
    let rat = match rat {
        "NR-5GC" | "NR5G-SA" | "NR5G-NSA" | "5G" => "NR",
        _ => rat,
    };
    let channel = numeric(channel, 0., if rat == "NR" { 3279165. } else { 262143. })
        .filter(|v| v.fract() == 0.)
        .map_or(Value::Null, |v| json!(v));
    let pci = as_num(&pci)
        .filter(|v| v.fract() == 0. && *v >= 0. && *v <= if rat == "NR" { 1007. } else { 503. })
        .map_or(Value::Null, |v| json!(v));
    let band_num = numeric(band.trim_start_matches(['n', 'N', 'b', 'B']), 1., 1024.)
        .filter(|v| v.fract() == 0.);
    let band_name = band_num.map(|v| format!("{}{v}", if rat == "NR" { "n" } else { "B" }));
    let candidates = if let Some(ref name) = band_name {
        vec![name.clone()]
    } else {
        as_num(&channel).map_or(vec![], |v| band_candidates(rat, v))
    };
    let mut row = json!({"rat":rat,"role":role,"roleKnown":role!="unknown","channel":channel,"pci":pci,"source":source,"band":band_name,"bandCandidates":candidates,"bandSource":if band_num.is_some(){"reported"}else{"arfcn-candidates"}});
    for (key, low, high) in [
        ("rsrp", -156., -28.),
        ("rsrq", -44., 20.),
        ("sinr", -23., 50.),
        ("rssi", -140., 0.),
        ("dlBandwidthMHz", 0.1, 1000.),
        ("ulBandwidthMHz", 0.1, 1000.),
    ] {
        row[key] = extras
            .get(key)
            .and_then(|v| {
                if v.is_string() {
                    numeric(text(v), low, high)
                } else {
                    as_num(v).filter(|v| v.is_finite() && *v >= low && *v <= high)
                }
            })
            .map_or(Value::Null, |v| json!(v));
    }
    if let Some(map) = extras.as_object() {
        for (key, value) in map {
            if ![
                "rsrp",
                "rsrq",
                "sinr",
                "rssi",
                "dlBandwidthMHz",
                "ulBandwidthMHz",
            ]
            .contains(&key.as_str())
            {
                row[key] = value.clone();
            }
        }
    }
    row
}

fn csv(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.trim().chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if field.trim().is_empty() => {
                field.clear();
                quoted = true;
            }
            ',' if !quoted => {
                fields.push(field.trim().to_owned());
                field.clear();
            }
            _ => field.push(c),
        }
    }
    fields.push(field.trim().to_owned());
    fields
}
pub(crate) fn records(raw: &str, prefix: &str) -> Vec<Vec<String>> {
    let lower = raw.to_ascii_lowercase();
    let needle = prefix.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut offset = 0;
    while let Some(start) = lower[offset..].find(&needle) {
        let start = offset + start + needle.len();
        let rest = &raw[start..];
        let end = rest
            .find(['\r', '\n'])
            .unwrap_or(rest.len())
            .min(lower[start..].find(&needle).unwrap_or(rest.len()));
        out.push(csv(&rest[..end]));
        offset = start + end;
    }
    out
}
fn deduplicate(rows: Vec<Value>) -> Vec<Value> {
    let mut merged: Vec<Value> = vec![];
    let mut positions = std::collections::HashMap::<_, usize>::new();
    for row in rows {
        if row["channel"].is_null() {
            continue;
        }
        let center = row["channelType"] == "center" && row["role"] != "neighbor";
        let identity = (
            text(&row["rat"]).to_owned(),
            text(&row["channelType"]).to_owned(),
            as_num(&row["channel"]).map(f64::to_bits),
            if center {
                None
            } else {
                as_num(&row["pci"]).map(f64::to_bits)
            },
        );
        let index = positions.get(&identity).copied();
        if let Some(index) = index {
            let previous = &merged[index];
            let stronger = as_num(&row["rsrp"])
                .is_some_and(|v| as_num(&previous["rsrp"]).is_none_or(|old| v > old));
            let (strongest, other) = if stronger {
                (&row, previous)
            } else {
                (previous, &row)
            };
            let mut value = strongest.clone();
            if let Some(map) = strongest.as_object() {
                for (key, v) in map {
                    if v.is_null() {
                        value[key] = other.get(key).cloned().unwrap_or(Value::Null);
                    }
                }
            }
            merged[index] = value;
        } else {
            positions.insert(identity, merged.len());
            merged.push(row);
        }
    }
    merged.sort_by(|a, b| {
        a["rsrp"]
            .is_null()
            .cmp(&b["rsrp"].is_null())
            .then_with(|| {
                as_num(&b["rsrp"])
                    .unwrap_or(-999.)
                    .total_cmp(&as_num(&a["rsrp"]).unwrap_or(-999.))
            })
            .then_with(|| text(&a["rat"]).cmp(text(&b["rat"])))
            .then_with(|| {
                as_num(&a["channel"])
                    .unwrap_or(0.)
                    .total_cmp(&as_num(&b["channel"]).unwrap_or(0.))
            })
    });
    merged
}
fn complete_inventory(serving: &[Value], mut carriers: Vec<Value>) -> Vec<Value> {
    for row in serving {
        if !carriers.iter().any(|other| other["rat"] == row["rat"]) {
            carriers.push(row.clone());
        }
    }
    deduplicate(carriers)
}
fn same_identity(a: &Value, b: &Value) -> bool {
    ["rat", "channel", "pci"].iter().all(|k| a[k] == b[k])
}
fn td_measured(mut row: Value) -> Value {
    let limits = if row["rat"] == "NR" {
        vec![
            ("rsrp", -156., -31.),
            ("rsrq", -43., 20.),
            ("sinr", -23., 40.),
        ]
    } else {
        vec![
            ("rsrp", -140., -44.),
            ("rsrq", -19.5, -3.),
            ("rssi", -90., -25.),
        ]
    };
    for (key, low, high) in limits {
        row[key] = as_num(&row[key])
            .filter(|v| *v >= low && *v <= high)
            .map_or(Value::Null, |v| json!(v));
    }
    row
}

fn nr_frequency_khz(channel: i64) -> Option<i64> {
    match channel {
        0..=599999 => Some(channel * 5),
        600000..=2016666 => Some(3000000 + (channel - 600000) * 15),
        2016667..=3279165 => Some(24250080 + (channel - 2016667) * 60),
        _ => None,
    }
}
fn nr_row_frequency(row: &Value) -> Option<i64> {
    as_num(&row["channel"])
        .filter(|v| v.fract() == 0. && *v >= 0. && *v <= 3279165.)
        .and_then(|v| nr_frequency_khz(v as i64))
}

fn associate_td_serving(serving: &mut [Value], carriers: &mut [Value]) {
    for primary in serving {
        let exact: Vec<_> = carriers
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                row["rat"] == primary["rat"]
                    && !primary["channel"].is_null()
                    && row["channel"] == primary["channel"]
            })
            .map(|(i, _)| i)
            .collect();
        let envelope: Vec<_> =
            if exact.is_empty() && primary["rat"] == "NR" && primary["channelType"] == "ssb" {
                let ssb = nr_row_frequency(primary);
                carriers
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| {
                        if row["rat"] != "NR" || row["channelType"] != "center" {
                            return false;
                        }
                        let center = nr_row_frequency(row);
                        let width = row["dlBandwidthMHz"]
                            .as_f64()
                            .filter(|v| *v > 0. && *v <= 400.);
                        match (ssb, center, width) {
                            (Some(s), Some(c), Some(w)) => ((s - c).abs() as f64) < w * 500.,
                            _ => false,
                        }
                    })
                    .map(|(i, _)| i)
                    .collect()
            } else {
                vec![]
            };
        let (matches, source) = if exact.is_empty() {
            (envelope, "unique-ssb-envelope")
        } else {
            (exact, "exact-channel")
        };
        if matches.len() != 1 {
            continue;
        }
        let row = &mut carriers[matches[0]];
        row["role"] = json!("pcc");
        row["roleKnown"] = json!(true);
        row["roleSource"] = json!(source);
        row["servingChannel"] = primary["channel"].clone();
        row["measurementChannel"] = primary["channel"].clone();
        row["measurementSource"] = json!("AT^MONSC");
        row["metricSources"] =
            json!({"rsrp":"AT^MONSC","rsrq":"AT^MONSC","sinr":"AT^MONSC","rssi":"AT^MONSC"});
        for key in ["pci", "rsrp", "rsrq", "sinr", "rssi"] {
            row[key] = primary[key].clone();
        }
        for key in [
            "band",
            "bandCandidates",
            "bandSource",
            "dlBandwidthMHz",
            "ulBandwidthMHz",
        ] {
            primary[key] = row[key].clone();
        }
        primary["carrierChannel"] = row["channel"].clone();
    }
}

fn tdtech(raw: &str) -> Value {
    let (mut serving, mut neighbors, mut carriers) = (vec![], vec![], vec![]);
    for p in records(raw, "^MONSC:") {
        if p.is_empty() {
            continue;
        }
        let rat = p[0].to_ascii_uppercase();
        if ["NR", "NR-5GC"].contains(&rat.as_str()) && p.len() >= 11 {
            serving.push(td_measured(cell("NR","pcc",&p[3],hex(&p[6]),"","AT^MONSC",json!({"channelType":"ssb","mcc":p[1],"mnc":p[2],"scsType":n(&p[4],0.,4.),"cellId":p[5],"tac":p[7],"rsrp":p[8],"rsrq":p[9],"sinr":p[10],"networkMode":"5G SA"}))));
        } else if ["LTE", "LTE-NR"].contains(&rat.as_str()) && p.len() >= 10 {
            serving.push(td_measured(cell("LTE","pcc",&p[3],hex(&p[5]),"","AT^MONSC",json!({"channelType":"center","mcc":p[1],"mnc":p[2],"cellId":p[4],"tac":p[6],"rsrp":p[7],"rsrq":p[8],"rssi":p[9],"networkMode":if rat=="LTE-NR"{"5G NSA"}else{"4G LTE"}}))));
        }
    }
    for p in records(raw, "^MONNC:") {
        if p.len() >= 3 && ["NR", "LTE"].contains(&p[0].as_str()) {
            let pci = hex(&p[2]);
            let metric = |index| p.get(index).map(String::as_str).unwrap_or("");
            let mut extra = json!({"rsrp":metric(3),"rsrq":metric(4)});
            extra[if p[0] == "NR" { "sinr" } else { "rssi" }] = json!(metric(5));
            if p[0] == "NR" {
                let scaled_rp = number(metric(3))
                    .filter(|v| v.fract() == 0. && *v < -200.)
                    .map(|rp| rp / 8.)
                    .filter(|rp| (-156. ..=-31.).contains(rp));
                if let Some(rp) = scaled_rp {
                    extra["rsrp"] = json!(rp);
                    for (key, index, low, high) in [("rsrq", 4, -43., 20.), ("sinr", 5, -23., 40.)]
                    {
                        extra[key] = json!(
                            number(metric(index))
                                .filter(|v| v.fract() == 0.)
                                .map(|v| v / 8.)
                                .filter(|v| (low..=high).contains(v))
                        );
                    }
                    extra["measurementEncoding"] = json!("eighth-db");
                }
            }
            neighbors.push(td_measured(cell(
                &p[0], "neighbor", &p[1], pci, "", "AT^MONNC", extra,
            )));
        }
    }
    for p in records(raw, "^HFREQINFO:") {
        if p.len() < 9 || !["6", "7"].contains(&p[1].as_str()) {
            continue;
        }
        let rat = if p[1] == "7" { "NR" } else { "LTE" };
        for chunk in p[2..].chunks_exact(7) {
            if numeric(&chunk[1], 1., f64::MAX).is_none() {
                continue;
            }
            let matched = serving
                .iter()
                .find(|r| r["rat"] == rat && as_num(&r["channel"]) == number(&chunk[1]));
            let mut extra = json!({"dlBandwidthMHz":number(&chunk[3]).unwrap_or(0.)/1000.,"ulBandwidthMHz":number(&chunk[6]).unwrap_or(0.)/1000.,"channelType":"center","bandwidthSource":if rat=="NR"{"connected-bwp"}else{"carrier"},"roleKnown":matched.is_some()});
            for key in ["rsrp", "rsrq", "sinr", "rssi"] {
                extra[key] = matched.map_or(Value::Null, |r| r[key].clone());
            }
            carriers.push(cell(
                rat,
                if matched.is_some() { "pcc" } else { "unknown" },
                &chunk[1],
                matched.map_or(Value::Null, |r| r["pci"].clone()),
                &chunk[0],
                "AT^HFREQINFO?",
                extra,
            ));
        }
    }
    carriers = deduplicate(carriers);
    associate_td_serving(&mut serving, &mut carriers);
    for primary in &mut serving {
        if let Some(matched) = carriers
            .iter()
            .find(|r| r["rat"] == primary["rat"] && r["channel"] == primary["channel"])
        {
            for k in [
                "band",
                "bandCandidates",
                "bandSource",
                "dlBandwidthMHz",
                "ulBandwidthMHz",
            ] {
                primary[k] = matched[k].clone();
            }
        } else if !carriers.iter().any(|r| r["rat"] == primary["rat"]) {
            carriers.push(primary.clone());
        } else {
            let bands: Vec<_> = carriers
                .iter()
                .filter(|r| r["rat"] == primary["rat"])
                .map(|r| r["band"].clone())
                .collect();
            if let Some(band) = bands
                .first()
                .filter(|b| !b.is_null() && bands.iter().all(|other| other == *b))
            {
                primary["band"] = band.clone();
                primary["bandCandidates"] = json!([band]);
                primary["bandSource"] = json!("carrier-inventory");
            }
        }
    }
    let pcc_rats: Vec<_> = carriers
        .iter()
        .filter(|r| r["role"] == "pcc")
        .map(|r| r["rat"].clone())
        .collect();
    for r in &mut carriers {
        if r["role"] == "unknown" && pcc_rats.contains(&r["rat"]) {
            r["role"] = json!("scc");
            r["roleKnown"] = json!(true);
        }
    }
    if serving.iter().any(|r| r["networkMode"] == "5G NSA") {
        for r in &mut serving {
            if r["rat"] == "NR" {
                r["networkMode"] = json!("5G NSA");
            }
        }
    }
    neighbors.retain(|r| {
        !serving
            .iter()
            .any(|s| !s["channel"].is_null() && !s["pci"].is_null() && same_identity(r, s))
    });
    json!({"serving":serving,"carriers":deduplicate(carriers),"neighbors":deduplicate(neighbors)})
}

fn lookup(code: Option<f64>, table: &[(i64, f64)]) -> Value {
    code.and_then(|v| {
        table
            .iter()
            .find(|(k, _)| *k as f64 == v)
            .map(|(_, bw)| *bw)
    })
    .map_or(Value::Null, |v| json!(v))
}
const LTE_BW: &[(i64, f64)] = &[(0, 1.4), (1, 3.), (2, 5.), (3, 10.), (4, 15.), (5, 20.)];
const NR_BW: &[(i64, f64)] = &[
    (0, 5.),
    (1, 10.),
    (2, 15.),
    (3, 20.),
    (4, 25.),
    (5, 30.),
    (6, 40.),
    (7, 50.),
    (8, 60.),
    (9, 70.),
    (10, 80.),
    (11, 90.),
    (12, 100.),
    (13, 200.),
    (14, 400.),
];
const FM_LTE: &[(i64, f64)] = &[
    (6, 1.4),
    (15, 3.),
    (25, 5.),
    (50, 10.),
    (75, 15.),
    (100, 20.),
];
const FM_NR: &[(i64, f64)] = &[
    (25, 5.),
    (50, 10.),
    (75, 15.),
    (100, 20.),
    (125, 25.),
    (150, 30.),
    (200, 40.),
    (250, 50.),
    (300, 60.),
    (400, 80.),
    (450, 90.),
    (500, 100.),
    (1000, 200.),
    (2000, 400.),
];
fn bw(code: &str, rat: &str) -> Value {
    if let Some(c) = MHZ.captures(code) {
        n(&c[1], 0.1, 1000.)
    } else {
        lookup(number(code), if rat == "NR" { NR_BW } else { LTE_BW })
    }
}
fn quectel(raw: &str, unisoc: bool) -> Value {
    let (mut serving, mut neighbors, mut carriers) = (vec![], vec![], vec![]);
    for mut p in records(raw, "+QENG:") {
        if p.first().is_some_and(|v| v.starts_with("neighbourcell")) {
            if !unisoc && p.len() == 7 && p[0] == "neighbourcell" && p[1] == "LTE" {
                neighbors.push(cell(
                    "LTE",
                    "neighbor",
                    &p[2],
                    n(&p[3], 0., 503.),
                    "",
                    "AT+QENG=neighbourcell",
                    json!({"rsrp":p[4],"rsrq":p[5],"reselectionRxLevel":p[6]}),
                ));
                continue;
            }
            if p.len()
                >= if p.get(1).is_some_and(|v| v == "LTE") {
                    8
                } else {
                    7
                }
                && ["LTE", "NR", "NR5G"].contains(&p[1].as_str())
            {
                let rat = if p[1] == "LTE" { "LTE" } else { "NR" };
                neighbors.push(cell(rat,"neighbor",&p[2],n(&p[3],0.,1007.),"","AT+QENG=neighbourcell",json!({"rsrp":p[if rat=="LTE"&&!unisoc{5}else{4}],"rsrq":p[if rat=="LTE"&&!unisoc{4}else{5}],"rssi":if rat=="LTE"&&!unisoc{json!(p[6])}else{Value::Null},"sinr":p[if rat=="LTE"&&(!unisoc||p[0]!="neighbourcell"){7}else{6}]})));
            }
            continue;
        }
        if p.first().is_some_and(|v| v == "servingcell") {
            p = p.into_iter().skip(2).collect();
        }
        if p.is_empty() {
            continue;
        }
        if p[0] == "LTE" && p.len() >= 15 {
            serving.push(cell("LTE","pcc",&p[6],n(&p[5],0.,503.),&p[7],"AT+QENG=servingcell",json!({"mcc":p[2],"mnc":p[3],"cellId":p[4],"tac":p[10],"ulBandwidthMHz":bw(&p[8],"LTE"),"dlBandwidthMHz":bw(&p[9],"LTE"),"rsrp":p[11],"rsrq":p[12],"rssi":p[13],"sinr":p[14],"networkMode":"4G LTE"})));
        } else if p[0] == "NR5G-SA" && p.len() >= 13 {
            serving.push(cell("NR","pcc",&p[7],n(&p[5],0.,1007.),&p[8],"AT+QENG=servingcell",json!({"mcc":p[2],"mnc":p[3],"cellId":p[4],"tac":p[6],"dlBandwidthMHz":if unisoc{n(&p[9],5.,400.)}else{bw(&p[9],"NR")},"rsrp":p[10],"rsrq":p[11],"sinr":p[12],"networkMode":"5G SA"})));
        } else if p[0] == "NR5G-NSA" && p.len() >= 9 {
            serving.push(cell("NR","nsa",&p[7],n(&p[3],0.,1007.),&p[8],"AT+QENG=servingcell",json!({"mcc":p[1],"mnc":p[2],"rsrp":p[4],"sinr":p[5],"rsrq":p[6],"dlBandwidthMHz":if p.len()>9&&!unisoc{bw(&p[9],"NR")}else{Value::Null},"cellId":if unisoc&&p.len()>10{json!(p[9])}else{Value::Null},"tac":if unisoc&&p.len()>10{json!(p[10])}else{Value::Null},"networkMode":"5G NSA"})));
        }
    }
    for p in records(raw, "+QCAINFO:") {
        if p.len() < 5 || !["PCC", "SCC"].contains(&p[0].as_str()) {
            continue;
        }
        let Some(band) = BAND.captures(&p[3]) else {
            continue;
        };
        let rat = if band[1].eq_ignore_ascii_case("NR5G") {
            "NR"
        } else {
            "LTE"
        };
        if numeric(&p[1], 0., if rat == "NR" { 3279165. } else { 262143. })
            .is_none_or(|v| v.fract() != 0.)
        {
            continue;
        }
        let reported_pci = if p.len() > 5 {
            n(&p[5], 0., 1007.)
        } else {
            Value::Null
        };
        if p.len() > 5
            && ((!p[5].is_empty() && p[5] != "-" && reported_pci.is_null())
                || reported_pci
                    .as_f64()
                    .is_some_and(|v| v.fract() != 0. || rat == "LTE" && v > 503.))
        {
            continue;
        }
        if !if p[0] == "PCC" {
            ["0", "1"].contains(&p[4].as_str())
        } else {
            ["0", "1", "2"].contains(&p[4].as_str())
        } {
            continue;
        }
        let matches = serving
            .iter()
            .filter(|r| {
                r["rat"] == rat
                    && as_num(&r["channel"]) == number(&p[1])
                    && (if reported_pci.is_null() {
                        p[0] == "PCC"
                    } else {
                        r["pci"] == reported_pci
                    })
            })
            .collect::<Vec<_>>();
        let primary = if matches.len() == 1 {
            Some(matches[0])
        } else {
            None
        };
        let pci = if reported_pci.is_null() {
            primary.map_or(Value::Null, |r| r["pci"].clone())
        } else {
            reported_pci
        };
        let bandwidth = if let Some(c) = MHZ.captures(&p[2]) {
            n(&c[1], 0.1, 400.)
        } else if rat == "LTE" {
            lookup(number(&p[2]), FM_LTE)
        } else {
            n(&p[2], 1., 400.)
        };
        let mut extras = json!({"dlBandwidthMHz":bandwidth,"active":p[4]==if p[0]=="PCC"{"1"}else{"2"},"metricSources":{}});
        for key in ["rsrp", "rsrq", "sinr", "rssi"] {
            extras[key] = primary.map_or(Value::Null, |r| r[key].clone());
            if !extras[key].is_null() {
                extras["metricSources"][key] = json!("AT+QENG=\"servingcell\"");
            }
        }
        if unisoc && rat == "LTE" && p.len() == 10 {
            extras["rsrp"] = json!(p[6]);
            extras["rsrq"] = json!(p[7]);
            extras["rssi"] = json!(p[8]);
            extras["sinr"] = numeric(&p[9], 0., 250.).map_or(Value::Null, |v| json!(v / 5. - 20.));
            for key in ["rsrp", "rsrq", "rssi", "sinr"] {
                extras["metricSources"][key] = json!("AT+QCAINFO");
            }
        } else if !unisoc && rat == "LTE" && p.len() == 10 {
            extras["rsrp"] = n(&p[6], -140., -44.);
            extras["rsrq"] = n(&p[7], -20., -3.);
            extras["rssi"] = n(&p[8], -140., 0.);
            extras["snr"] = n(&p[9], -10., 30.);
            for key in ["rsrp", "rsrq", "rssi", "snr"] {
                extras["metricSources"][key] = json!("AT+QCAINFO");
            }
        }
        carriers.push(cell(
            rat,
            if p[0] == "PCC" { "pcc" } else { "scc" },
            &p[1],
            pci,
            &band[2],
            "AT+QCAINFO",
            extras,
        ));
    }
    let mut serving = deduplicate(serving);
    if serving.iter().any(|r| r["networkMode"] == "5G NSA") {
        for row in &mut serving {
            if row["rat"] == "LTE" {
                row["networkMode"] = json!("5G NSA");
            }
        }
    }
    neighbors.retain(|r| !serving.iter().any(|s| same_identity(s, r)));
    json!({"carriers":complete_inventory(&serving,carriers),"serving":serving,"neighbors":deduplicate(neighbors)})
}

fn simcom(raw: &str) -> Value {
    let mut serving = vec![];
    let integer = |value: &str, max| numeric(value, 0., max).filter(|v| v.fract() == 0.);
    let nr_snr = |value: &str| {
        numeric(value, -230., 400.)
            .filter(|v| !(-23.0..=40.0).contains(v))
            .map_or(Value::Null, |v| json!(v / 10.))
    };
    let legacy_power = |value: &str| {
        numeric(value, -156., -28.)
            .map(|v| (v, 1.))
            .or_else(|| numeric(value, -1560., -280.).map(|v| (v / 10., 10.)))
    };
    for mut p in records(raw, "+CPSI:") {
        if p.is_empty() {
            continue;
        }
        let rat = p[0].clone();
        if rat == "NR5G" && p.len() == 6 {
            if integer(&p[1], 1007.).is_none() || integer(&p[2], 3279165.).is_none() {
                continue;
            }
            let power = legacy_power(&p[4]);
            serving.push(cell(
                "NR",
                "nsa",
                &p[2],
                n(&p[1], 0., 1007.),
                "",
                "AT+CPSI?",
                json!({"rsrq":power.map_or(Value::Null,|(_,scale)|scaled(&p[3],scale)),"rsrp":power.map_or(Value::Null,|(v,_)|json!(v)),
                    "snr":nr_snr(&p[5]),"snrRaw":p[5],"snrEncoding":"firmware-version-dependent",
                    "networkMode":"5G NSA","channelType":"ssb-or-center",
                    "protocolLayout":"SIM8200-V1.00.01-NSA"}),
            ));
            continue;
        }
        if rat == "NR5G" && p.len() == 10 && p[1] == "Online" {
            let Some(plmn) = PLMN.captures(&p[2]) else {
                continue;
            };
            if integer(&p[5], 1007.).is_none() || integer(&p[6], 3279165.).is_none() {
                continue;
            }
            let power = legacy_power(&p[7]);
            serving.push(cell(
                "NR",
                "pcc",
                &p[6],
                n(&p[5], 0., 1007.),
                "",
                "AT+CPSI?",
                json!({"mcc":&plmn[1],"mnc":&plmn[2],"tac":p[3],"cellId":p[4],
                    "rsrp":power.map_or(Value::Null,|(v,_)|json!(v)),"rsrq":power.map_or(Value::Null,|(_,scale)|scaled(&p[8],scale)),
                    "snr":nr_snr(&p[9]),"snrRaw":p[9],"snrEncoding":"firmware-version-dependent",
                    "networkMode":"5G SA","protocolLayout":"SIM8200-V1.00.01-SA"}),
            ));
            continue;
        }
        if rat == "NR5G_NSA" && [7, 9].contains(&p.len()) {
            if integer(&p[1], 1007.).is_none() || integer(&p[3], 3279165.).is_none() {
                continue;
            }
            if let Some(band) = SIM_BAND
                .captures(&p[2])
                .filter(|_| p[2].to_ascii_uppercase().starts_with("NR5G"))
            {
                serving.push(cell("NR","nsa",&p[3],n(&p[1],0.,1007.),&band[1],"AT+CPSI?",json!({"rsrp":scaled(&p[4],10.),"rsrq":scaled(&p[5],10.),"snr":nr_snr(&p[6]),"snrRaw":p[6],"snrEncoding":"firmware-version-dependent","networkMode":"5G NSA","channelType":"ssb-or-center"})));
            }
            continue;
        }
        if !["LTE", "NR5G_SA"].contains(&rat.as_str()) || p.len() < 11 || p[1] != "Online" {
            continue;
        }
        if p[2].len() == 3
            && p[2].bytes().all(|c| c.is_ascii_digit())
            && [2, 3].contains(&p[3].len())
            && p[3].bytes().all(|c| c.is_ascii_digit())
        {
            let joined = format!("{}-{}", p[2], p[3]);
            p.splice(2..4, [joined]);
        }
        let (Some(plmn), Some(band)) = (PLMN.captures(&p[2]), SIM_BAND.captures(&p[6])) else {
            continue;
        };
        if p.len() != if rat == "LTE" { 14 } else { 11 } {
            continue;
        }
        if integer(&p[5], if rat == "LTE" { 503. } else { 1007. }).is_none()
            || integer(&p[7], if rat == "LTE" { 262143. } else { 3279165. }).is_none()
            || !p[6]
                .to_ascii_uppercase()
                .starts_with(if rat == "LTE" { "EUTRAN" } else { "NR5G" })
        {
            continue;
        }
        let mut extras = json!({"mcc":&plmn[1],"mnc":&plmn[2],"tac":p[3],"cellId":p[4],"rsrp":scaled(&p[if rat=="LTE"{11}else{8}],10.),"rsrq":scaled(&p[if rat=="LTE"{10}else{9}],10.),"sinr":if rat=="LTE"{n(&p[13],0.,30.)}else{Value::Null},"networkMode":if rat=="LTE"{"4G LTE"}else{"5G SA"}});
        if rat != "LTE" {
            extras["snr"] = nr_snr(&p[10]);
            extras["snrRaw"] = json!(p[10]);
            extras["snrEncoding"] = json!("firmware-version-dependent");
        }
        if rat == "LTE" {
            extras["dlBandwidthMHz"] = bw(&p[8], "LTE");
            extras["ulBandwidthMHz"] = bw(&p[9], "LTE");
            extras["rssi"] = scaled(&p[12], 10.);
        }
        serving.push(cell(
            if rat == "LTE" { "LTE" } else { "NR" },
            "pcc",
            &p[7],
            n(&p[5], 0., 1007.),
            &band[1],
            "AT+CPSI?",
            extras,
        ));
    }
    let mut serving = deduplicate(serving);
    if serving.iter().any(|row| row["networkMode"] == "5G NSA") {
        for row in &mut serving {
            if row["rat"] == "LTE" {
                row["networkMode"] = json!("5G NSA");
            }
        }
    }
    json!({"serving":serving,"carriers":serving,"neighbors":[]})
}
fn generic(raw: &str) -> Value {
    let mut rssi = Value::Null;
    for p in records(raw, "+CSQ:") {
        if let Some(v) = p
            .first()
            .and_then(|v| numeric(v, 0., 31.))
            .filter(|v| v.fract() == 0.)
        {
            rssi = json!(-113. + 2. * v);
        }
    }
    json!({"serving":[],"carriers":[],"neighbors":[],"rssi":rssi})
}

fn fm_band(value: &str, expected: Option<&str>) -> Option<(&'static str, i64)> {
    let v = value.trim();
    let (rat, band) = if FM_BAND.is_match(v) {
        let band = v[2..].parse::<i64>().ok()?;
        if !(1..=512).contains(&band) {
            return None;
        }
        ("NR", band)
    } else if v.bytes().all(|b| b.is_ascii_digit()) {
        let band = v.parse::<i64>().ok()?;
        if !(101..=171).contains(&band) {
            return None;
        }
        ("LTE", band - 100)
    } else {
        return None;
    };
    if expected.is_none_or(|r| r == rat) {
        Some((rat, band))
    } else {
        None
    }
}
fn quantized(value: &str, rat: &str, metric: &str) -> Option<(f64, Value)> {
    let (maximum, low, step) = match (rat, metric) {
        ("NR", "rsrp") => (125., -156., 1.),
        ("NR", "rsrq") => (126., -43., 0.5),
        ("NR", "sinr") => (126., -23., 0.5),
        ("LTE", "rsrp") => (96., -140., 1.),
        ("LTE", "rsrq") => (33., -19.5, 0.5),
        ("LTE", "rssi") => (62., -110., 1.),
        _ => return None,
    };
    let index = numeric(value, 1., maximum)?;
    if index.fract() != 0. {
        return None;
    }
    let v = low + (index - 1.) * step;
    Some((
        v,
        json!({"lower":v,"upperExclusive":v+step,"encoding":"quantized-index"}),
    ))
}
fn blocks<'a>(raw: &'a str, prefix: &str) -> Vec<&'a str> {
    let lower = raw.to_ascii_lowercase();
    let mut out = vec![];
    let mut offset = 0;
    while let Some(start) = lower[offset..].find(&prefix.to_ascii_lowercase()) {
        let start = offset + start + prefix.len();
        let rest = &raw[start..];
        let mut end = rest.len();
        let mut at = 0;
        for line in rest.split_inclusive('\n') {
            let t = line.trim_start().to_ascii_uppercase();
            if at > 0
                && (t.starts_with("AT")
                    || t.starts_with("+CESQ:")
                    || t.starts_with(if prefix == "+GTCCINFO:" {
                        "+GTCAINFO:"
                    } else {
                        "+GTCCINFO:"
                    }))
            {
                end = at;
                break;
            }
            at += line.len();
        }
        out.push(&rest[..end]);
        offset = start + end;
    }
    out
}
fn supplement(row: &mut Value, metric: &str, value: &str, rat: &str, source: &str) {
    if let Some((v, interval)) = quantized(value, rat, metric) {
        row[metric] = json!(v);
        row["metricSources"][metric] = json!(source);
        row["metricIntervals"][metric] = interval;
    }
}
fn fibocom_address(value: &str, hexadecimal: bool) -> String {
    if !hexadecimal {
        return value.to_owned();
    }
    let value = value
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    if value.is_empty() || value.len() > 10 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return String::new();
    }
    u64::from_str_radix(value, 16)
        .ok()
        .map(|v| v.to_string())
        .unwrap_or_default()
}
fn complete_gt_block(raw: &str) -> bool {
    let lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    lines.last() == Some(&"OK")
        && !lines
            .iter()
            .any(|s| *s == "ERROR" || s.starts_with("+CME ERROR") || s.starts_with("+CMS ERROR"))
}
fn fm350(raw: &str, hexadecimal: bool, nr_neighbor_layout: bool) -> Value {
    let (mut serving, mut neighbors, mut carriers) = (vec![], vec![], vec![]);
    let widths = |rat: &str| {
        if hexadecimal {
            if rat == "NR" { NR_BW } else { LTE_BW }
        } else if rat == "NR" {
            FM_NR
        } else {
            FM_LTE
        }
    };
    for block in blocks(raw, "+GTCCINFO:") {
        if !complete_gt_block(block) {
            continue;
        }
        for line in block.lines() {
            let mut p = csv(line);
            if p.len() < 2
                || !["1", "2"].contains(&p[0].as_str())
                || !["4", "9"].contains(&p[1].as_str())
            {
                continue;
            }
            let primary = p[0] == "1";
            if p.len() != if primary { 14 } else { 12 } {
                continue;
            }
            let rat = if p[1] == "9" { "NR" } else { "LTE" };
            if hexadecimal {
                for i in [4, 5, 6, 7] {
                    p[i] = fibocom_address(&p[i], true);
                }
            }
            let band = if primary {
                fm_band(&p[8], Some(rat)).map_or(String::new(), |(_, b)| b.to_string())
            } else {
                String::new()
            };
            let mut row = cell(
                rat,
                if primary { "pcc" } else { "neighbor" },
                &p[6],
                n(&p[7], 0., 1007.),
                &band,
                "AT+GTCCINFO?",
                json!({"mcc":p[2],"mnc":p[3],"tac":p[4],"cellId":p[5],"metricSources":{},"metricIntervals":{}}),
            );
            if row["channel"].is_null() || row["pci"].is_null() {
                continue;
            }
            if !(hexadecimal && !primary && rat == "NR") {
                supplement(
                    &mut row,
                    "rsrp",
                    &p[if primary { 12 } else { 10 }],
                    rat,
                    "AT+GTCCINFO?",
                );
                supplement(
                    &mut row,
                    "rsrq",
                    &p[if primary { 13 } else { 11 }],
                    rat,
                    "AT+GTCCINFO?",
                );
            }
            if nr_neighbor_layout && !primary && rat == "NR" {
                supplement(&mut row, "rsrp", &p[11], "NR", "AT+GTCCINFO?");
                supplement(&mut row, "sinr", &p[9], "NR", "AT+GTCCINFO?");
                if let Some((_, band)) = fm_band(&p[8], Some("NR")) {
                    row["band"] = json!(format!("n{band}"));
                    row["bandCandidates"] = json!([format!("n{band}")]);
                    row["bandSource"] = json!("reported");
                }
            }
            if primary {
                if hexadecimal && rat == "NR" {
                    supplement(&mut row, "sinr", &p[10], "NR", "AT+GTCCINFO?");
                }
                row["dlBandwidthMHz"] = lookup(number(&p[9]), widths(rat));
                if !row["dlBandwidthMHz"].is_null() {
                    row["metricSources"]["dlBandwidthMHz"] = json!("AT+GTCCINFO?");
                }
                serving.push(row);
            } else {
                neighbors.push(row);
            }
        }
    }
    serving = deduplicate(serving);
    let nsa = serving.iter().any(|r| r["rat"] == "NR") && serving.iter().any(|r| r["rat"] == "LTE");
    for r in &mut serving {
        r["networkMode"] = json!(if nsa {
            "5G NSA"
        } else if r["rat"] == "NR" {
            "5G SA"
        } else {
            "4G LTE"
        });
    }
    for p in records(raw, "+CESQ:") {
        if p.len() != 9 {
            continue;
        }
        let counts = (
            serving.iter().filter(|r| r["rat"] == "NR").count(),
            serving.iter().filter(|r| r["rat"] == "LTE").count(),
        );
        for r in &mut serving {
            let nr = r["rat"] == "NR";
            if (if nr { counts.0 } else { counts.1 }) != 1 {
                continue;
            }
            let mut indices = if nr {
                vec![("rsrq", 6), ("rsrp", 7), ("sinr", 8)]
            } else {
                vec![("rsrq", 4), ("rsrp", 5)]
            };
            if !nsa {
                indices.push(("rssi", 0));
            }
            for (metric, index) in indices {
                supplement(
                    r,
                    metric,
                    &p[index],
                    if metric == "rssi" || !nr { "LTE" } else { "NR" },
                    "AT+CESQ",
                );
            }
        }
    }
    for block in blocks(raw, "+GTCAINFO:") {
        if !complete_gt_block(block) {
            continue;
        }
        for line in block.lines() {
            let Some(c) = GT_CA.captures(line) else {
                continue;
            };
            let mut p = csv(&c[2]);
            let primary = c[1].eq_ignore_ascii_case("PCC");
            if p.len() != if primary { 9 } else { 12 } {
                continue;
            }
            let o = if primary { 0 } else { 2 };
            if hexadecimal {
                for i in [o + 1, o + 2] {
                    p[i] = fibocom_address(&p[i], true);
                }
            }
            let Some((rat, band)) = fm_band(&p[o], None) else {
                continue;
            };
            let mut row = cell(
                rat,
                if primary { "pcc" } else { "scc" },
                &p[o + 2],
                n(&p[o + 1], 0., 1007.),
                &band.to_string(),
                "AT+GTCAINFO?",
                json!({"roleKnown":true,"active":primary||p[0]=="2","dlBandwidthMHz":lookup(number(&p[o+3]),widths(rat)),"ulBandwidthMHz":if !primary&&p[1]=="1"{lookup(number(&p[6]),widths(rat))}else{Value::Null}}),
            );
            if row["channel"].is_null() || row["pci"].is_null() {
                continue;
            }
            if primary {
                if let Some(actual) = serving.iter_mut().find(|s| same_identity(s, &row)) {
                    if !row["dlBandwidthMHz"].is_null() {
                        actual["dlBandwidthMHz"] = row["dlBandwidthMHz"].clone();
                        actual["metricSources"]["dlBandwidthMHz"] = json!("AT+GTCAINFO?");
                    }
                    for key in [
                        "rsrp",
                        "rsrq",
                        "sinr",
                        "rssi",
                        "metricSources",
                        "metricIntervals",
                    ] {
                        row[key] = actual[key].clone();
                    }
                }
            }
            carriers.push(row);
        }
    }
    neighbors.retain(|r| !serving.iter().any(|s| same_identity(s, r)));
    json!({"carriers":complete_inventory(&serving,carriers),"serving":serving,"neighbors":deduplicate(neighbors)})
}

pub fn parse(parser: &str, raw: &str) -> Result<Value> {
    if raw.len() > MAX_AT_RESPONSE {
        return Err(PanelError::new("AT_FAILED", 503));
    }
    dispatch(parser, raw)
}
pub(crate) fn parse_snapshot(parser: &str, raw: &str) -> Result<Value> {
    if raw.len() > MAX_AT_RESPONSE * crate::MAX_SAMPLE_QUERIES + crate::MAX_SAMPLE_QUERIES - 1 {
        return Err(PanelError::new("AT_FAILED", 503));
    }
    dispatch(parser, raw)
}
fn dispatch(parser: &str, raw: &str) -> Result<Value> {
    let mut result = match parser {
        "tdtech" => tdtech(raw),
        "quectel" => quectel(raw, false),
        "quectel-rg801" => quectel(&vendor_csv::frames(raw).join("\n"), false),
        "quectel-unisoc" => quectel(raw, true),
        "simcom" => simcom(raw),
        "fibocom-fm350" => fm350(raw, false, false),
        "fibocom-qualcomm" => fm350(raw, true, false),
        "fibocom-nl668" => fibocom_lte::parse(raw),
        "fibocom-hex" => fm350(raw, true, true),
        "huawei-unisoc" => vendor_csv::huawei(raw),
        "meig-cellinfo" => vendor_csv::meig(raw),
        "neoway-netdmsgex" => vendor_csv::neoway(raw),
        "gosuncn-lte" => vendor_labels::gosuncn(raw, true),
        "gosuncn-qualcomm" => vendor_labels::gosuncn(raw, false),
        "foxconn-plus" => vendor_labels::debug(raw, "AT+debug?", false),
        "foxconn-caret" => vendor_labels::debug(raw, "AT^debug?", false),
        "thales-debug" => vendor_labels::debug(raw, "AT^DEBUG?", true),
        "telit-cainfoext" => vendor_labels::telit(raw),
        "openluat-air724" => openluat::parse(raw),
        "sierra-em91" => sierra::parse(raw, false),
        "sierra-em92" => sierra::parse(raw, true),
        "sierra-lte" => sierra_lte::parse(raw),
        "generic" => generic(raw),
        _ => return Err(PanelError::new("PARSER_UNSUPPORTED", 422)),
    };
    crate::session_qos::attach(parser, raw, &mut result);
    crate::network_operator::attach(raw, &mut result);
    Ok(result)
}
