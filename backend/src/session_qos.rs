//! 参考QModem公开会话查询格式，按上下文关联APN、签约速率和QCI。
use super::radio::{numeric, records};
use serde_json::{Value, json};

pub fn slow_query(command: &str) -> bool {
    matches!(
        command,
        "AT^DSAMBR=1"
            | "AT^DSAMBR=8"
            | "AT+CGEQOSRDP"
            | "AT+CGCONTRDP"
            | "AT+QNWCFG=\"nr5g_ambr\""
            | "AT^IMSSWITCH?"
    )
}
fn cid(s: &str) -> Option<u64> {
    s.parse::<u64>().ok().filter(|v| (1..=255).contains(v))
}
fn apn(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && !s.eq_ignore_ascii_case("ims")
        && !s.to_ascii_lowercase().starts_with("ims.")
}
pub fn attach(parser: &str, raw: &str, result: &mut Value) {
    let mut contracts = vec![];
    let mut active: Vec<_> = records(raw, "+CGCONTRDP:")
        .into_iter()
        .filter(|p| p.len() >= 3 && cid(&p[0]).is_some() && apn(&p[2]))
        .map(|p| (cid(&p[0]).unwrap_or(0), p[2].to_ascii_lowercase()))
        .collect();
    active.sort();
    active.dedup();
    if parser == "tdtech" {
        for p in records(raw, "^DSAMBR:") {
            if !(3..=5).contains(&p.len()) {
                continue;
            }
            let Some(id) = cid(&p[0]).filter(|v| [1, 8].contains(v)) else {
                continue;
            };
            if p.len() >= 4 && !apn(&p[3]) {
                continue;
            }
            if let (Some(dl), Some(ul)) = (numeric(&p[1], 1., 1e10), numeric(&p[2], 1., 1e10)) {
                contracts.push(json!({"dlKbps":dl,"ulKbps":ul,"cid":id,"apn":p.get(3),"source":format!("AT^DSAMBR={id}"),"kind":"AMBR"}));
            }
        }
    } else if matches!(parser, "quectel" | "quectel-rg650") {
        for p in records(raw, "+QNWCFG:") {
            if p.len() != 6 || p[0] != "nr5g_ambr" || !apn(&p[1]) {
                continue;
            }
            if let (Some(dl), Some(ul)) = (numeric(&p[3], 0.001, 1e7), numeric(&p[5], 0.001, 1e7)) {
                contracts.push(json!({"dlKbps":dl*1000.,"ulKbps":ul*1000.,"apn":p[1],"source":"AT+QNWCFG=\"nr5g_ambr\"","kind":"AMBR","cqiDl":numeric(&p[2],0.,15.),"cqiUl":numeric(&p[4],0.,15.)}));
            }
        }
    }
    if contracts.is_empty() {
        for p in records(raw, "+CGEQOSRDP:") {
            if p.len() != 8 {
                continue;
            }
            let Some(id) = cid(&p[0]) else {
                continue;
            };
            let names: Vec<_> = active.iter().filter(|(key, _)| *key == id).collect();
            if names.len() != 1 {
                continue;
            }
            if let (Some(dl), Some(ul)) = (numeric(&p[6], 1., 1e10), numeric(&p[7], 1., 1e10)) {
                contracts.push(json!({"dlKbps":dl,"ulKbps":ul,"cid":id,"apn":names[0].1,"source":"AT+CGEQOSRDP","kind":"APN-AMBR"}));
            }
        }
    }
    contracts.sort_by_key(|v| v["cid"].as_u64().unwrap_or(0));
    contracts.dedup();
    if contracts.len() == 1 {
        result["subscription"] = contracts[0].clone();
    } else if contracts.len() > 1 {
        result["subscriptionAmbiguous"] = json!(true);
    }
    result["subscriptionContexts"] = json!(contracts);
    let subscription_cid = result["subscription"]["cid"].as_u64().or_else(|| {
        let candidates: Vec<_> = active
            .iter()
            .filter(|p| {
                result["subscription"]["apn"]
                    .as_str()
                    .is_none_or(|s| s.eq_ignore_ascii_case(&p.1))
            })
            .collect();
        (candidates.len() == 1)
            .then(|| Some(candidates[0].0))
            .flatten()
    });
    let mut qos = vec![];
    for p in records(raw, "+CGEQOSRDP:") {
        if p.len() < 2 {
            continue;
        }
        let Some(id) = cid(&p[0]) else {
            continue;
        };
        let Some(qci) = p[1].parse::<u64>().ok().filter(|n| (1..=255).contains(n)) else {
            continue;
        };
        qos.push(json!({"cid":id,"qci":qci,"source":"AT+CGEQOSRDP"}));
    }
    qos.sort_by_key(|v| v["cid"].as_u64().unwrap_or(0));
    qos.dedup();
    if let Some(id) = subscription_cid {
        let matching: Vec<_> = qos.iter().filter(|v| v["cid"] == id).collect();
        if matching.len() == 1 {
            result["sessionQos"] = matching[0].clone();
        }
    }
    result["qosContexts"] = json!(qos);
    if let Some(config) = crate::sms_settings::parse(raw) {
        result["smsReceiving"] = config.public();
    }
}
