//! 运营商查询与公共网络标识归一化，不用未知MNC推断运营商品牌。
use serde_json::{Value, json};
fn digits(value: &Value, min: usize, max: usize) -> Option<String> {
    let raw = value.as_str()?.trim();
    (raw.len() >= min && raw.len() <= max && raw.bytes().all(|b| b.is_ascii_digit()))
        .then(|| raw.to_owned())
}
fn name(mcc: &str, mnc: &str) -> String {
    if mcc == "460" {
        match mnc {
            "00" | "02" | "07" | "08" => return "中国移动".into(),
            "01" | "06" | "09" => return "中国联通".into(),
            "03" | "05" | "11" => return "中国电信".into(),
            "15" => return "中国广电".into(),
            _ => {}
        }
    }
    format!("{mcc}{mnc}")
}
fn text_name(raw: &str) -> String {
    match raw.to_ascii_uppercase().replace([' ', '_'], "-").as_str() {
        "CHN-UNICOM" | "CHINA-UNICOM" | "UNICOM" => "中国联通".into(),
        "CHN-CT" | "CHN-TELECOM" | "CHINA-TELECOM" => "中国电信".into(),
        "CHINA-MOBILE" | "CHN-CMCC" | "CMCC" => "中国移动".into(),
        _ => raw.to_owned(),
    }
}
pub fn attach(raw: &str, result: &mut Value) {
    let mut plmns: Vec<_> = result["serving"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let mcc = digits(&r["mcc"], 3, 3)?;
            let mnc = digits(&r["mnc"], 1, 3)?;
            let mnc = if mnc.len() == 1 {
                format!("0{mnc}")
            } else {
                mnc
            };
            Some((mcc, mnc, r["source"].as_str().unwrap_or("").to_owned()))
        })
        .collect();
    plmns.sort();
    plmns.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    let cops = crate::radio::records(raw, "+COPS:");
    let cops = cops.first().filter(|_| cops.len() == 1).filter(|p| {
        (3..=4).contains(&p.len())
            && p[0] != "2"
            && ["0", "1", "2", "3", "4"].contains(&p[0].as_str())
            && ["0", "1", "2"].contains(&p[1].as_str())
            && !p[2].is_empty()
            && p[2].len() <= 128
            && !p[2].chars().any(char::is_control)
    });
    let numeric = cops
        .filter(|p| {
            p[1] == "2" && (5..=6).contains(&p[2].len()) && p[2].bytes().all(|b| b.is_ascii_digit())
        })
        .map(|p| (p[2][..3].to_owned(), p[2][3..].to_owned()));
    if plmns.len() > 1 {
        result["operatorConflict"] = json!(true);
        return;
    }
    let identity = plmns
        .first()
        .cloned()
        .or_else(|| numeric.clone().map(|(a, b)| (a, b, "AT+COPS?".into())));
    let value = if let Some((mcc, mnc, source)) = identity {
        let conflict = numeric
            .as_ref()
            .is_some_and(|(a, b)| *a != mcc || *b != mnc);
        let mapped = name(&mcc, &mnc);
        let textual = (mapped == format!("{mcc}{mnc}") && !conflict)
            .then(|| cops.filter(|p| p[1] != "2"))
            .flatten();
        let label = textual.map(|p| text_name(&p[2])).unwrap_or(mapped);
        json!({"name":label,"mcc":mcc,"mnc":mnc,"source":if textual.is_some(){"AT+COPS?"}else{&source},"meaning":"registered-network-not-sim-home","conflict":conflict})
    } else if let Some(c) = cops.filter(|p| p[1] != "2") {
        json!({"name":text_name(&c[2]),"source":"AT+COPS?","meaning":"registered-network-not-sim-home"})
    } else {
        return;
    };
    result["operator"] = value["name"].clone();
    result["networkOperator"] = value;
}
