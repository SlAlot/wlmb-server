//! 各厂商标签式状态回复解析；字段缺失保留为空，不拼接为额外载波。
use super::{
    vendor_csv::{empty, frames},
    *,
};
use std::collections::BTreeMap;
static LABEL: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?i)(?:^|[\s,])([a-z0-9_]+):\s*"));
static COMPONENT: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?m)^\s*(PCC|SCC\d{1,2})-"));
type Fields = BTreeMap<String, String>;
fn fields(raw: &str) -> Option<Fields> {
    let matches: Vec<_> = LABEL.captures_iter(raw).collect();
    if matches.len() > 128 {
        return None;
    }
    let mut out = Fields::new();
    for (i, c) in matches.iter().enumerate() {
        let start = c.get(0)?.end();
        let end = matches
            .get(i + 1)
            .and_then(|c| c.get(0))
            .map_or(raw.len(), |m| m.start());
        let v = raw[start..end].trim_matches(|c: char| c.is_whitespace() || c == ',');
        if v.len() > 256 || out.insert(c[1].to_ascii_lowercase(), v.into()).is_some() {
            return None;
        }
    }
    Some(out)
}
fn get<'a>(f: &'a Fields, key: &str) -> &'a str {
    f.get(key).map_or("", String::as_str)
}
fn scalar(raw: &str) -> &str {
    let raw = raw.trim();
    for suffix in ["MHz", "mhz", "dBm", "dbm", "dB", "db"] {
        if let Some(v) = raw.strip_suffix(suffix) {
            return v.trim();
        }
    }
    raw
}
fn mean(raw: &str, low: f64, high: f64) -> Value {
    let parts: Vec<_> = scalar(raw)
        .split(|c: char| c.is_whitespace() || c == ',' || c == '/')
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() || parts.len() > 8 {
        return Value::Null;
    }
    let Some(values) = parts
        .iter()
        .map(|p| numeric(p, low, high))
        .collect::<Option<Vec<_>>>()
    else {
        return Value::Null;
    };
    json!(values.iter().sum::<f64>() / values.len() as f64)
}
fn debug_row(f: &Fields, rat: &str, role: &str, source: &str, thales: bool) -> Value {
    let prefix = if rat == "NR" { "nr" } else { "lte" };
    let key = |suffix| format!("{prefix}_{suffix}");
    cell(
        rat,
        role,
        scalar(get(f, "channel")),
        n(scalar(get(f, "pci")), 0., 1007.),
        scalar(get(f, &key("band"))),
        source,
        json!({
            "mcc":get(f,"mcc"),"mnc":get(f,"mnc"),"cellId":get(f,&key("cell_id")),"tac":get(f,&key("tac")),
            "rsrp":mean(get(f,if rat=="LTE"&&thales{"lte_rsrp"}else{"rsrp"}),-156.,-28.),
            "rsrq":mean(get(f,"rsrq"),-44.,20.),"sinr":mean(get(f,&key("snr")),-23.,50.),
            "rssi":mean(get(f,&key("rssi")),-140.,0.),
            "dlBandwidthMHz":n(scalar(get(f,&key("band_width"))),0.1,400.),
            "networkMode":if rat=="NR"{"5G SA"}else if get(f,"rat")=="LTE+NR"{"5G NSA"}else{"4G LTE"},"roleKnown":true}),
    )
}
pub(super) fn debug(raw: &str, source: &str, thales: bool) -> Value {
    let mut serving = vec![];
    let mut carriers = vec![];
    for frame in frames(raw) {
        let payload = frame
            .lines()
            .filter(|l| !l.trim().starts_with("AT") && l.trim() != "OK")
            .collect::<Vec<_>>()
            .join("\n");
        let mut chunks = payload.split("nr_scc1:");
        let Some(f) = fields(chunks.next().unwrap_or("")) else {
            continue;
        };
        let rat = match get(&f, "rat") {
            "NR5G_SA" => "NR",
            "LTE" | "LTE+NR" => "LTE",
            _ => continue,
        };
        let row = debug_row(&f, rat, "pcc", source, thales);
        if row["channel"].is_null() || row["pci"].is_null() {
            continue;
        }
        serving.push(row.clone());
        carriers.push(row);
        if !thales && rat == "NR" {
            if let Some(chunk) = chunks.next().filter(|_| chunks.next().is_none()) {
                if let Some(f) = fields(chunk) {
                    let row = debug_row(&f, rat, "scc", source, thales);
                    if !row["band"].is_null() && !row["dlBandwidthMHz"].is_null() {
                        carriers.push(row);
                    }
                }
            }
        }
    }
    if serving.len() != 1 {
        return empty();
    }
    if carriers.len() > 1
        && carriers[0]["channel"] == carriers[1]["channel"]
        && !carriers[1]["channel"].is_null()
    {
        carriers.truncate(1);
    }
    json!({"serving":serving,"carriers":carriers,"neighbors":[]})
}
fn telit_band(s: &str) -> Option<(&'static str, i64)> {
    let k = s.parse::<usize>().ok()?;
    const LTE: &[(usize, i64)] = &[
        (120, 1),
        (121, 2),
        (122, 3),
        (123, 4),
        (124, 5),
        (125, 6),
        (126, 7),
        (127, 8),
        (128, 9),
        (129, 10),
        (130, 11),
        (131, 12),
        (132, 13),
        (133, 14),
        (134, 17),
        (135, 33),
        (136, 34),
        (137, 35),
        (138, 36),
        (139, 37),
        (140, 38),
        (141, 39),
        (142, 40),
        (143, 18),
        (144, 19),
        (145, 20),
        (146, 21),
        (147, 24),
        (148, 25),
        (149, 41),
        (150, 42),
        (151, 43),
        (152, 23),
        (153, 26),
        (154, 32),
        (155, 125),
        (156, 126),
        (157, 127),
        (158, 28),
        (159, 29),
        (160, 30),
        (161, 66),
        (162, 250),
        (163, 46),
        (166, 71),
        (167, 47),
        (168, 48),
    ];
    const NR: &[i64] = &[
        1, 2, 3, 5, 7, 8, 20, 28, 38, 41, 50, 51, 66, 70, 71, 74, 75, 76, 77, 78, 79, 80, 81, 82,
        83, 84, 85, 257, 258, 259, 260, 261, 12, 25, 34, 39, 40, 65, 86, 48, 14, 13, 18, 26, 30,
        29, 53,
    ];
    LTE.iter()
        .find(|(code, _)| *code == k)
        .map(|(_, band)| ("LTE", *band))
        .or_else(|| {
            k.checked_sub(250)
                .and_then(|i| NR.get(i))
                .map(|b| ("NR", *b))
        })
}
pub(super) fn telit(raw: &str) -> Value {
    let mut serving = vec![];
    let mut carriers = vec![];
    let mut reported = None;
    for frame in frames(raw) {
        let header = records(frame, "#CAINFOEXT:");
        if header.len() != 1 || header[0].len() != 2 {
            continue;
        }
        let Some(count) = numeric(&header[0][0], 1., 16.).filter(|v| v.fract() == 0.) else {
            continue;
        };
        reported = Some(count);
        let starts: Vec<_> = COMPONENT.captures_iter(frame).collect();
        if starts.len() > 16 {
            continue;
        }
        let mut roles = std::collections::BTreeSet::new();
        for (i, c) in starts.iter().enumerate() {
            let start = c.get(0).map_or(0, |m| m.end());
            let end = starts
                .get(i + 1)
                .and_then(|c| c.get(0))
                .map_or(frame.len(), |m| m.start());
            let text = frame[start..end]
                .lines()
                .filter(|l| l.trim() != "OK")
                .collect::<Vec<_>>()
                .join("\n");
            let Some(f) = fields(&text) else {
                continue;
            };
            let Some((rat, band)) = telit_band(get(&f, "bandclass")) else {
                continue;
            };
            let primary = &c[1] == "PCC";
            if !roles.insert(c[1].to_owned()) {
                return empty();
            }
            let width = if !get(&f, "bw").is_empty() {
                n(scalar(get(&f, "bw")), 0.1, 400.)
            } else if rat == "LTE" {
                lookup(number(get(&f, "dl_bw")), LTE_BW)
            } else {
                Value::Null
            };
            let channel = if get(&f, "ch").is_empty() {
                get(&f, "rx_ch")
            } else {
                get(&f, "ch")
            };
            let row = cell(
                rat,
                if primary { "pcc" } else { "scc" },
                channel,
                n(get(&f, "pci"), 0., 1007.),
                &band.to_string(),
                "AT#CAINFOEXT?",
                json!({"rsrp":get(&f,"rsrp"),"rsrq":get(&f,"rsrq"),"rssi":get(&f,"rssi"),
                    "sinr":numeric(get(&f,"sinr"),0.,250.).map(|v|-20.+v*0.2),"dlBandwidthMHz":width,
                    "tac":get(&f,"tac"),"roleKnown":true,"networkMode":if rat=="NR"{"5G SA"}else{"4G LTE"}}),
            );
            if row["channel"].is_null() || row["pci"].is_null() {
                continue;
            }
            if primary {
                serving.push(row.clone());
            }
            carriers.push(row);
        }
    }
    if serving.len() != 1 {
        return empty();
    }
    if carriers.iter().any(|r| r["rat"] == "NR") && carriers.iter().any(|r| r["rat"] == "LTE") {
        for row in serving.iter_mut().chain(carriers.iter_mut()) {
            row["networkMode"] = json!("5G NSA");
        }
    }
    let carriers = deduplicate(carriers);
    json!({"inventoryIncomplete":reported.is_some_and(|n|n!=carriers.len() as f64),"reportedCarrierCount":reported,
        "serving":serving,"carriers":carriers,"neighbors":[]})
}
pub(super) fn gosuncn(raw: &str, lte: bool) -> Value {
    let mut serving = vec![];
    for frame in frames(raw) {
        for line in frame.lines() {
            let line = line.trim();
            let Some(payload) = line.strip_prefix("+ZCELLINFO:").map(str::trim) else {
                continue;
            };
            let Some((first, rest)) = payload.split_once(',') else {
                continue;
            };
            let Some(f) = fields(rest) else {
                continue;
            };
            let rat = if lte {
                "LTE"
            } else {
                match first.trim() {
                    "NR5G" => "NR",
                    "LTE" => "LTE",
                    _ => continue,
                }
            };
            let row = cell(
                rat,
                "pcc",
                if lte { "" } else { get(&f, "freq") },
                n(get(&f, "pci"), 0., 1007.),
                get(&f, "band"),
                "AT+ZCELLINFO?",
                json!({"mcc":get(&f,"mcc"),"mnc":get(&f,"mnc"),"tac":if lte{first.trim()}else{get(&f,"tac")},
                    "cellId":if get(&f,"cellid").is_empty(){get(&f,"cell_id")}else{get(&f,"cellid")},
                    "rsrp":get(&f,"rsrp"),"rsrq":get(&f,"rsrq"),"rssi":get(&f,"rssi"),"sinr":get(&f,"sinr"),
                    "networkMode":if rat=="NR"{"5G SA"}else{"4G LTE"},"identityUnavailable":lte}),
            );
            if !lte && (row["channel"].is_null() || row["pci"].is_null()) {
                continue;
            }
            serving.push(row);
        }
    }
    if serving.len() != 1 {
        return empty();
    }
    if lte {
        let row = &mut serving[0];
        row["metricSources"] = json!({});
        row["metricIntervals"] = json!({});
        for frame in frames(raw) {
            let cesq = records(frame, "+CESQ:");
            if cesq.len() == 1 && cesq[0].len() == 6 {
                for (key, i, max, base, step) in
                    [("rsrp", 5, 97., -141., 1.), ("rsrq", 4, 34., -19.5, 0.5)]
                {
                    if let Some(v) = numeric(&cesq[0][i], 1., max - 1.).filter(|v| v.fract() == 0.)
                    {
                        let low = base + v * step;
                        row[key] = json!(low + step / 2.);
                        row["metricIntervals"][key] =
                            json!({"lower":low,"upper":low+step,"upperInclusive":false});
                        row["metricSources"][key] = json!("AT+CESQ");
                    }
                }
            }
            let csq = records(frame, "+CSQ:");
            if csq.len() == 1 && csq[0].len() == 2 {
                if let Some(v) = numeric(&csq[0][0], 1., 30.).filter(|v| v.fract() == 0.) {
                    row["rssi"] = json!(-113. + v * 2.);
                    row["metricSources"]["rssi"] = json!("AT+CSQ");
                }
            }
        }
    } else {
        for frame in frames(raw) {
            for line in frame.lines().filter(|l| l.contains("narfcn:")) {
                if let Some(f) = fields(line) {
                    let row = cell(
                        "NR",
                        "unknown",
                        get(&f, "narfcn"),
                        n(get(&f, "nr5g_pci"), 0., 1007.),
                        get(&f, "band"),
                        "AT+ZCELLINFO?",
                        json!({"rsrp":get(&f,"5g_rsrp"),"rsrq":get(&f,"5g_rsrq"),"sinr":get(&f,"5g_sinr"),
                            "cellId":get(&f,"cell_id"),"networkMode":"5G NSA"}),
                    );
                    if serving[0]["rat"] == "LTE"
                        && !row["channel"].is_null()
                        && !row["pci"].is_null()
                    {
                        serving[0]["networkMode"] = json!("5G NSA");
                        serving.push(row);
                        break;
                    }
                }
            }
        }
    }
    json!({"serving":serving,"carriers":[],"neighbors":[]})
}
