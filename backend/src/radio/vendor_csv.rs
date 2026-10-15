//! 各厂商CSV式回复字段的有界解析，按已识别平台选择格式。
use super::*;

pub(super) fn frames(raw: &str) -> Vec<&str> {
    let mut start = 0;
    let mut offset = 0;
    let mut out = Vec::new();
    for line in raw.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with("AT") {
            start = offset;
        }
        offset += line.len();
        if t == "OK" {
            if out.len() < crate::MAX_SAMPLE_QUERIES {
                out.push(&raw[start..offset]);
            }
            start = offset;
        } else if t == "ERROR" || t.starts_with("+CME ERROR") || t.starts_with("+CMS ERROR") {
            start = offset;
        }
    }
    out
}
pub(super) fn empty() -> Value {
    json!({"serving":[],"carriers":[],"neighbors":[]})
}
fn value(p: &[String], i: usize) -> &str {
    p.get(i).map_or("", String::as_str)
}
fn one(raw: &str, prefix: &str) -> Option<Vec<String>> {
    let rows: Vec<_> = frames(raw)
        .iter()
        .flat_map(|f| records(f, prefix))
        .collect();
    (rows.len() == 1).then(|| rows[0].clone())
}
pub(super) fn huawei(raw: &str) -> Value {
    let raw = frames(raw).join("\n");
    let mut result = tdtech(&raw);
    for group in ["serving", "carriers"] {
        if let Some(rows) = result[group].as_array_mut() {
            for row in rows.iter_mut().filter(|r| r["rat"] == "LTE") {
                row["rssi"] = Value::Null;
            }
        }
    }
    result["neighbors"] = json!([]);
    if let Some(p) = one(&raw, "^CSERSSI:") {
        if p.len() >= 14
            && result["serving"]
                .as_array()
                .is_some_and(|a| a.len() == 1 && a[0]["rat"] == "LTE")
        {
            let row = cell(
                "NR",
                "unknown",
                "",
                Value::Null,
                "",
                "AT^CSERSSI?",
                json!({"rsrp":p[11],"rsrq":p[12],"sinr":p[13],"networkMode":"5G NSA","identityUnavailable":true}),
            );
            if !row["rsrp"].is_null() {
                result["secondaryMeasurement"] = row;
                result["serving"][0]["networkMode"] = json!("5G NSA");
            }
        }
    }
    result
}
pub(super) fn meig(raw: &str) -> Value {
    let Some(p) = one(raw, "^CELLINFO:") else {
        return empty();
    };
    if p.len() < 17 || p.len() > 64 {
        return empty();
    }
    let rat = value(&p, 0);
    let nr = rat == "5G";
    if !nr && !["LTE", "LTE-NR", "eMTC", "NB-IoT"].contains(&rat) {
        return empty();
    }
    let width = number(value(&p, if nr { 8 } else { 10 }))
        .filter(|v| v.fract() == 0.)
        .map(|v| if nr { v } else { v / 5. });
    let row = cell(
        if nr { "NR" } else { "LTE" },
        "pcc",
        "",
        n(value(&p, 5), 0., 1007.),
        value(&p, if nr { 7 } else { 9 }),
        "AT^CELLINFO=1",
        json!({
            "mcc":p[2],"mnc":p[3],"cellId":value(&p,if nr{4}else{7}),
            "tac":value(&p,if nr{6}else{8}),"rsrp":p[14],"rsrq":p[15],"sinr":scaled(&p[16],10.),
            "dlBandwidthMHz":width,"ulBandwidthMHz":if nr {None} else {width},
            "scsType":if nr {n(value(&p,9),0.,4.)} else {Value::Null},
            "networkMode":if nr {"5G SA"} else if rat=="LTE-NR" {"5G NSA"} else {"4G LTE"},
            "identityUnavailable":true
        }),
    );
    let mut result = json!({"serving":[row],"carriers":[],"neighbors":[]});
    if rat == "LTE-NR" && p.len() >= 33 {
        let row = cell(
            "NR",
            "unknown",
            "",
            Value::Null,
            value(&p, 32),
            "AT^CELLINFO=1",
            json!({"rsrp":value(&p,29),"rsrq":value(&p,30),"sinr":scaled(value(&p,31),10.),
                "dlBandwidthMHz":n(value(&p,34),0.1,400.),"networkMode":"5G NSA","identityUnavailable":true}),
        );
        result["secondaryMeasurement"] = row;
    }
    result
}
pub(super) fn neoway(raw: &str) -> Value {
    let Some(p) = one(raw, "+NETDMSGEX:") else {
        return empty();
    };
    if p.len() < 9 || p.len() > 24 {
        return empty();
    }
    let rat = match p[0].as_str() {
        "NR to 5GCN" => "NR",
        "NR to EPS" | "NR-LTE ENDC" | "NR-LTE NEDC" => "NR",
        "LTE" => "LTE",
        _ => return empty(),
    };
    if rat == "LTE" && p.len() < 13 {
        return empty();
    }
    let Some((mcc, mnc)) = p[1].split_once('+').filter(|(m, c)| {
        m.len() == 3
            && (2..=3).contains(&c.len())
            && m.bytes().chain(c.bytes()).all(|b| b.is_ascii_digit())
    }) else {
        return empty();
    };
    let bandwidth = |i| {
        number(value(&p, i))
            .filter(|v| v.fract() == 0.)
            .and_then(|v| lookup(Some(v), LTE_BW).as_f64())
    };
    let row = cell(
        rat,
        "pcc",
        &p[3],
        n(value(&p, if rat == "NR" { 5 } else { 6 }), 0., 1007.),
        &p[2],
        "AT+NETDMSGEX",
        json!({"mcc":mcc,"mnc":mnc,"cellId":if rat=="LTE"{json!(p[5])}else{Value::Null},
            "gnbId":if rat=="NR"{json!(p[4])}else{Value::Null},"tac":if rat=="LTE"{json!(p[4])}else{Value::Null},
            "rsrp":scaled(value(&p,if rat=="NR"{6}else{9}),10.),
            "rsrq":scaled(value(&p,if rat=="NR"{7}else{10}),10.),
            "sinr":scaled(value(&p,if rat=="NR"{8}else{11}),10.),
            "rssi":if rat=="LTE"{scaled(&p[12],10.)}else{Value::Null},
            "dlBandwidthMHz":if rat=="LTE"{bandwidth(16)}else{None},
            "ulBandwidthMHz":if rat=="LTE"{bandwidth(17)}else{None},
            "networkMode":if p[0]=="NR to 5GCN"{"5G SA"}else if rat=="NR"{"5G NSA"}else{"4G LTE"}}),
    );
    if row["channel"].is_null() || row["pci"].is_null() {
        return empty();
    }
    json!({"serving":[row],"carriers":[],"neighbors":[]})
}
