//! 参考 QModem 公开 send_at 契约；只借用传输，自有解析与独立模式共用，禁止任意 AT 透传。
use crate::{
    PanelError, Result, catalog, modem_inventory, radio,
    sampling::{self, Sample},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

const COMMANDS: &[&str] = &[
    "AT^MONSC",
    "AT^HFREQINFO?",
    "AT^MONNC",
    "AT+QENG=\"servingcell\"",
    "AT+QCAINFO",
    "AT+QENG=\"neighbourcell\"",
    "AT+GTCCINFO?",
    "AT+GTCAINFO?",
    "AT!NRINFO?",
    "AT!LTEINFO?",
    "AT+CCED=0,1",
    "AT+CCED=0,2",
];
fn error(code: &'static str) -> PanelError {
    PanelError::new(code, 503)
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
pub fn style(schema: &Value) -> Option<u64> {
    if schema["send_at"]["config_section"] != 3 {
        return None;
    }
    schema["send_at"]["params"]
        .as_u64()
        .filter(|v| [2, 3].contains(v))
}
pub fn command_allowed(command: &str) -> bool {
    COMMANDS.contains(&command)
}

pub fn request(row: &Value, style: u64, command: &str) -> Result<Value> {
    let key = row["id"]
        .as_str()
        .filter(|v| identifier(v))
        .ok_or_else(|| error("MODEM_CHANGED"))?;
    let port = row["port"]
        .as_str()
        .filter(|v| modem_inventory::valid_port(v))
        .ok_or_else(|| error("MODEM_CHANGED"))?;
    let use_ubus = row["qmodemUseUbus"]
        .as_bool()
        .ok_or_else(|| error("QMODEM_AT_CONFIG_INVALID"))?;
    if !command_allowed(command)
        || plan(row).is_none_or(|plan| !plan.commands.iter().any(|v| v == command))
    {
        return Err(error("AT_WRITE_DISABLED"));
    }
    let params = json!({"at":command,"port":port,"use_ubus_flag":if use_ubus {"1"}else{"0"}});
    let params = match style {
        2 => params,
        3 => json!(params.to_string()),
        _ => return Err(error("QMODEM_AT_ABI_UNSUPPORTED")),
    };
    Ok(json!({"config_section":key,"params":params}))
}
pub fn request_valid(request: &Value) -> bool {
    if request.as_object().is_none_or(|v| v.len() != 2)
        || request["config_section"]
            .as_str()
            .is_none_or(|v| !identifier(v))
    {
        return false;
    }
    let parsed;
    let params = if let Some(raw) = request["params"].as_str() {
        if raw.len() > 512 {
            return false;
        }
        parsed = match serde_json::from_str::<Value>(raw) {
            Ok(v) if serde_json::to_vec(&v).is_ok_and(|canonical| canonical == raw.as_bytes()) => v,
            _ => return false,
        };
        &parsed
    } else {
        &request["params"]
    };
    params.as_object().is_some_and(|v| v.len() == 3)
        && params["at"].as_str().is_some_and(command_allowed)
        && params["port"]
            .as_str()
            .is_some_and(modem_inventory::valid_port)
        && matches!(params["use_ubus_flag"].as_str(), Some("0" | "1"))
}
pub fn response(reply: &Value, row: &Value, command: &str) -> Result<String> {
    let payload = &reply["at_cfg"];
    if payload["status"] != "1" {
        return Err(error("AT_FAILED"));
    }
    let raw = payload["res"]
        .as_str()
        .filter(|v| v.len() <= crate::MAX_AT_RESPONSE && !v.contains('\0'))
        .ok_or_else(|| error("AT_RESPONSE_MISMATCH"))?;
    let echo = format!("at {} {command}", row["port"].as_str().unwrap_or(""));
    if payload["cmd"] != echo {
        return Err(error("AT_RESPONSE_MISMATCH"));
    }
    let lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect();
    // 忙碌、掉线或执行失败不是能力声明，必须停止本轮，避免继续向不确定通道排队。
    if lines.iter().any(|v| {
        ["NO CARRIER", "NO DIALTONE", "BUSY"].contains(v)
            || v.starts_with("+CME ERROR:")
            || v.starts_with("+CMS ERROR:")
    }) {
        return Err(error("AT_FAILED"));
    }
    if lines
        .iter()
        .any(|v| ["ERROR", "COMMAND NOT SUPPORT"].contains(v))
    {
        return Err(PanelError::new("AT_UNSUPPORTED", 422));
    }
    let finals = lines.iter().filter(|v| **v == "OK").count();
    let expected = command
        .strip_prefix("AT")
        .unwrap_or("")
        .split(['?', '='])
        .next()
        .unwrap_or("");
    // sms_tool_q 在成功退出前移除 OK；仅接受已绑定工具的成功回执与对应查询标签，不把空串补成成功。
    let helper_final = row["qmodemAtTool"] == "sms_tool_q"
        && finals == 0
        && lines.iter().any(|line| {
            line.split_once(':')
                .is_some_and(|(tag, _)| tag.trim() == expected)
        });
    if !(finals == 1 && lines.last() == Some(&"OK") || helper_final)
        || !sampling::reply_matches_query(command, raw)
        || lines
            .iter()
            .any(|line| !sampling::reply_matches_query(command, line))
    {
        return Err(error("AT_RESPONSE_MISMATCH"));
    }
    for line in &lines {
        let line = line.to_ascii_lowercase();
        if line.starts_with("+qeng:") {
            let neighbor = line.contains("\"neighbourcell");
            if command == "AT+QENG=\"neighbourcell\"" && !neighbor
                || command == "AT+QENG=\"servingcell\"" && neighbor
            {
                return Err(error("AT_RESPONSE_MISMATCH"));
            }
        }
    }
    let normalized = if helper_final {
        format!("{raw}\nOK\n")
    } else {
        raw.to_owned()
    };
    if normalized.len() > crate::MAX_AT_RESPONSE {
        return Err(error("AT_RESPONSE_MISMATCH"));
    }
    Ok(normalized)
}

pub struct Plan {
    pub commands: Vec<String>,
    pub neighbors: bool,
    pub carriers: bool,
}
pub fn plan(row: &Value) -> Option<Plan> {
    let model = row["model"].as_str()?;
    let profile = catalog::identify(model);
    let family = profile["family"].as_str()?;
    let vendor = row["manufacturer"].as_str().unwrap_or("");
    if !vendor.is_empty()
        && !vendor.eq_ignore_ascii_case(family)
        && !(family == "tdtech" && vendor.eq_ignore_ascii_case("huawei"))
    {
        return None;
    }
    let platform = row["platform"].as_str().unwrap_or("");
    if !platform.is_empty() && profile["platform"] != platform {
        return None;
    }
    let caps = catalog::radio_capabilities(model);
    if caps["neighbors"] != true {
        return None;
    }
    let commands: Vec<_> = profile["queries"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter(|v| command_allowed(v))
        .map(str::to_owned)
        .collect();
    if commands.is_empty() || commands.len() > 3 {
        return None;
    }
    Some(Plan {
        commands,
        neighbors: caps["neighbors"] == true,
        carriers: caps["carrierAggregation"] == true,
    })
}
fn identity(value: &Value) -> Option<Vec<String>> {
    let rows = value["serving"].as_array()?;
    if rows.is_empty()
        || rows
            .iter()
            .any(|row| row["channel"].is_null() || row["pci"].is_null() && row["cellId"].is_null())
    {
        return None;
    }
    let mut keys: Vec<_> = rows
        .iter()
        .map(|row| {
            json!([
                row["rat"],
                row["channel"],
                row["pci"],
                row["cellId"],
                row["tac"],
                row["mcc"],
                row["mnc"]
            ])
            .to_string()
        })
        .collect();
    keys.sort();
    Some(keys)
}
fn inventory_states(value: &mut Value, plan: &Plan, complete: bool, at: u64) {
    let source = "qmodem.send_at";
    value["carrierState"] = json!({"status":if complete && plan.carriers {"reported"}else{"serving-only"},
        "inventoryComplete":false,"source":source,"observedAtMs":at,"stale":false});
    value["neighborState"] = json!({"status":if !plan.neighbors {"unsupported"}else if !complete {"pending"}
        else if value["neighbors"].as_array().is_none_or(Vec::is_empty) {"empty"}else{"reported"},
        "source":source,"observedAtMs":at,"stale":false});
    if !complete {
        if plan.carriers {
            value["carrierState"]["queryStatus"] = json!("pending");
        }
        if plan.neighbors {
            value["neighborState"]["queryStatus"] = json!("pending");
        }
    }
}
pub fn collect(
    row: &Value,
    mut query: impl FnMut(&str) -> Result<String>,
    mut publish: impl FnMut(Value) -> Result<()>,
) -> Result<Value> {
    let plan = plan(row).ok_or_else(|| error("QMODEM_AT_PROFILE_UNSUPPORTED"))?;
    let mut modem = row.clone();
    modem["profile"] = catalog::identify(row["model"].as_str().unwrap_or(""));
    let parser = modem["profile"]["parser"]
        .as_str()
        .ok_or_else(|| error("PARSER_UNSUPPORTED"))?;
    let mut samples = BTreeMap::new();
    let mut errors = vec![];
    let mut anchor = None;
    let mut primary = vec![];
    let mut uncertain = false;
    for command in &plan.commands {
        let at = crate::events::wall_ms();
        let response = query(command).and_then(|raw| {
            let parsed = radio::parse(parser, &raw)?;
            let unknown_ca = command == "AT+QCAINFO"
                && raw.contains("+QCAINFO:")
                && parsed["carriers"].as_array().is_none_or(Vec::is_empty);
            let unknown_neighbors = command == "AT+QENG=\"neighbourcell\""
                && raw.lines().any(|line| {
                    line.trim().starts_with("+QENG:")
                        && ["\"LTE\"", "\"NR\"", "\"NR5G\""]
                            .iter()
                            .any(|rat| line.contains(rat))
                })
                && parsed["neighbors"].as_array().is_none_or(Vec::is_empty);
            if unknown_ca || unknown_neighbors {
                return Err(error("AT_PARSE_UNAVAILABLE"));
            }
            Ok((raw, parsed))
        });
        match response {
            Ok((raw, parsed)) => {
                if identity(&parsed).is_some() {
                    primary.push(command.clone());
                }
                samples.insert(
                    command.clone(),
                    Sample {
                        raw,
                        wall_ms: at,
                        mono: Instant::now(),
                    },
                );
            }
            Err(e) => {
                errors.push(json!({"command":command,"code":e.code}));
                let alternate_primary = parser == "sierra-em91" || parser == "sierra-em92";
                if anchor.is_none() && (!alternate_primary || e.code != "AT_UNSUPPORTED") {
                    return Err(e);
                }
                if e.code == "AT_UNSUPPORTED" || e.code == "AT_PARSE_UNAVAILABLE" {
                    continue;
                }
                // 超时或错帧后不继续下一个查询，不在同一轮重试或绕过通道。
                uncertain = true;
                break;
            }
        }
        let mut stage = sampling::publish(
            &modem,
            &plan.commands,
            &samples,
            &errors,
            crate::events::wall_ms(),
        )?;
        if anchor.is_none() {
            if let Some(keys) = identity(&stage) {
                anchor = Some(keys);
                stage["neighbors"] = json!([]);
                stage["carriers"] = stage["serving"].clone();
                inventory_states(&mut stage, &plan, false, at);
                stage["pending"] = json!(true);
                stage["partial"] = json!(true);
                publish(stage)?;
            } else if parser != "sierra-em91" && parser != "sierra-em92" {
                return Err(error("QMODEM_AT_PRIMARY_INVALID"));
            }
        }
    }
    if anchor.is_none() {
        return Err(error("QMODEM_AT_PRIMARY_INVALID"));
    }
    let before = sampling::publish(
        &modem,
        &plan.commands,
        &samples,
        &errors,
        crate::events::wall_ms(),
    )?;
    // 读取前后核验真实服务小区；邻区与载波在核验完成前不对外发布。
    if !uncertain {
        let mut verified = BTreeMap::new();
        for command in &primary {
            let at = crate::events::wall_ms();
            let raw = query(command)?;
            verified.insert(
                command.clone(),
                Sample {
                    raw,
                    wall_ms: at,
                    mono: Instant::now(),
                },
            );
        }
        let checked = sampling::publish(
            &modem,
            &plan.commands,
            &verified,
            &[],
            crate::events::wall_ms(),
        )?;
        if identity(&checked).is_none() || identity(&checked) != identity(&before) {
            return Err(error("QMODEM_AT_SERVING_CHANGED"));
        }
    }
    let mut value = sampling::publish(
        &modem,
        &plan.commands,
        &samples,
        &errors,
        crate::events::wall_ms(),
    )?;
    let at = value["observedAtMs"]
        .as_u64()
        .unwrap_or_else(crate::events::wall_ms);
    let carriers: BTreeSet<_> = value["carriers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| !row["channel"].is_null() && !row["pci"].is_null())
        .map(|row| json!([row["rat"], row["channel"], row["pci"]]).to_string())
        .collect();
    if let Some(rows) = value["neighbors"].as_array_mut() {
        rows.retain(|row| {
            !carriers.contains(&json!([row["rat"], row["channel"], row["pci"]]).to_string())
        });
    }
    inventory_states(&mut value, &plan, true, at);
    for (field, rows) in [("carrierState", "carriers"), ("neighborState", "neighbors")] {
        let failure = errors.iter().rev().find(|e| {
            let command = e["command"].as_str().unwrap_or("");
            uncertain
                || if field == "neighborState" {
                    neighbor_command(command)
                } else {
                    carrier_command(command)
                }
        });
        if let Some(failure) = failure {
            value[rows] = if rows == "carriers" {
                value["serving"].clone()
            } else {
                json!([])
            };
            value[field]["queryStatus"] = json!("error");
            value[field]["code"] = failure["code"].clone();
            value[field]["status"] = json!(if rows == "carriers" {
                "serving-only"
            } else {
                "unavailable"
            });
        }
    }
    if uncertain {
        value["infoSupplementDeferred"] = json!(true);
    }
    if row["qmodemAtTool"] == "sms_tool_q" {
        value["completionSource"] = json!("qmodem-helper-status");
    }
    Ok(value)
}
fn neighbor_command(command: &str) -> bool {
    [
        "AT^MONNC",
        "AT+QENG=\"neighbourcell\"",
        "AT+GTCCINFO?",
        "AT!NRINFO?",
        "AT!LTEINFO?",
        "AT+CCED=0,2",
    ]
    .contains(&command)
}
fn carrier_command(command: &str) -> bool {
    [
        "AT^HFREQINFO?",
        "AT+QCAINFO",
        "AT+GTCAINFO?",
        "AT!NRINFO?",
        "AT!LTEINFO?",
    ]
    .contains(&command)
}
