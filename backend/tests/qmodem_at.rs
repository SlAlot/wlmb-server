#![allow(clippy::unwrap_used)]
// QModem 只传输固定查询；解析、协议与异常回归均使用合成回复。
use serde_json::{Value, json};
use wlmb_server::{PanelError, qmodem_at};
const SERVING: &str = "+QENG: \"servingcell\",\"NOCONN\",\"NR5G-SA\",\"TDD\",460,01,1234AB,214,ABCD,627264,78,12,-85,-10,19,1,-\r\nOK\r\n";
const CA: &str = "+QCAINFO: \"PCC\",627264,100,\"NR5G BAND 78\",1,214\r\n+QCAINFO: \"SCC\",636664,80,\"NR5G BAND 78\",2,215\r\nOK\r\n";
const NEIGHBOR: &str = "+QENG: \"neighbourcell\",\"NR5G\",627264,30,-95,-13,5\r\n+QENG: \"neighbourcell intra\",\"LTE\",1650,40,-12,-97,-70,7\r\nOK\r\n";
fn row() -> Value {
    json!({"id":"2_1","model":"rm520f-gl","port":"/dev/ttyUSB2","qmodemUseUbus":true})
}
fn reply(command: &str, raw: &str) -> Value {
    json!({"at_cfg":{"status":"1","cmd":format!("at /dev/ttyUSB2 {command}"),"res":raw}})
}
fn raw(command: &str) -> String {
    match command {
        "AT+QENG=\"servingcell\"" => SERVING,
        "AT+QCAINFO" => CA,
        "AT+QENG=\"neighbourcell\"" => NEIGHBOR,
        _ => panic!("不应发送计划以外的命令"),
    }
    .into()
}
#[test]
fn abi_follows_actual_string_or_table_signature_not_version_or_type_label() {
    for style in [2, 3] {
        let schema = json!({"send_at":{"config_section":3,"params":style}});
        assert_eq!(qmodem_at::style(&schema), Some(style));
        let request = qmodem_at::request(&row(), style, "AT+QCAINFO").unwrap();
        assert_eq!(request["params"].is_string(), style == 3);
        assert!(qmodem_at::request_valid(&request));
    }
    for schema in [
        json!({}),
        json!({"send_at":{"config_section":5,"params":3}}),
        json!({"send_at":{"config_section":3,"params":8}}),
    ] {
        assert_eq!(qmodem_at::style(&schema), None);
    }
    for command in [
        "AT+QSCAN=3,1",
        "AT+CFUN=0",
        "AT+CMGS=3",
        "AT+QNWLOCK=\"common/5g\",1,2,3",
        "AT+QCAINFO\rAT+CFUN=0",
        "AT+CGSN",
    ] {
        assert!(qmodem_at::request(&row(), 3, command).is_err());
    }
}
#[test]
fn request_guards_reject_arbitrary_ports_shell_options_unknown_fields_and_noncanonical_strings() {
    for key in ["id", "port"] {
        let mut device = row();
        device[key] = json!("../../unsafe; reboot");
        assert!(qmodem_at::request(&device, 3, "AT+QCAINFO").is_err());
    }
    let request = qmodem_at::request(&row(), 2, "AT+QCAINFO").unwrap();
    for (key, value) in [
        ("extra", json!(true)),
        ("port", json!("/tmp/ttyUSB2")),
        ("use_ubus_flag", json!("1 -M")),
        ("at", json!("AT+CFUN=0")),
    ] {
        let mut invalid = request.clone();
        invalid["params"][key] = value;
        assert!(!qmodem_at::request_valid(&invalid));
    }
    for encoded in [
        "{\"at\":\"AT+QCAINFO\",\"at\":\"AT+CFUN=0\",\"port\":\"/dev/ttyUSB2\",\"use_ubus_flag\":\"1\"}",
        "{ \"at\":\"AT+QCAINFO\",\"port\":\"/dev/ttyUSB2\",\"use_ubus_flag\":\"1\"}",
        "not-json",
    ] {
        let mut invalid = request.clone();
        invalid["params"] = json!(encoded);
        assert!(!qmodem_at::request_valid(&invalid));
    }
}
#[test]
fn replies_require_matching_command_and_complete_uncrossed_frame() {
    let command = "AT+QENG=\"neighbourcell\"";
    assert_eq!(
        qmodem_at::response(&reply(command, NEIGHBOR), &row(), command).unwrap(),
        NEIGHBOR
    );
    for raw in [
        "OK\nOK\n",
        "OK\0",
        "+QENG: \"neighbourcell\",\"NR5G\",627264,30,-95,-13,5",
        SERVING,
        "+QCAINFO: \"PCC\",627264,100,\"NR5G BAND 78\",1,214\nOK\n",
        "+QENG: \"neighbourcell\",\"NR5G\",627264,30,-95,-13,5\n+QCAINFO: x\nOK\n",
    ] {
        assert!(qmodem_at::response(&reply(command, raw), &row(), command).is_err());
    }
    assert_eq!(
        qmodem_at::response(&reply(command, "ERROR\n"), &row(), command)
            .unwrap_err()
            .code,
        "AT_UNSUPPORTED"
    );
    let mut invalid = reply(command, NEIGHBOR);
    invalid["at_cfg"]["cmd"] = json!("at /dev/ttyUSB9 AT+QCAINFO");
    assert!(qmodem_at::response(&invalid, &row(), command).is_err());
}
#[test]
fn own_parser_reads_both_neighbor_rats_and_carriers_before_and_after_identity_check() {
    let mut calls = vec![];
    let mut stages = vec![];
    let value = qmodem_at::collect(
        &row(),
        |command| {
            calls.push(command.to_owned());
            Ok(raw(command))
        },
        |stage| {
            stages.push(stage);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls.first(), calls.last());
    assert_eq!(stages.len(), 1);
    assert!(stages[0]["neighbors"].as_array().unwrap().is_empty());
    assert_eq!(stages[0]["serving"][0]["rsrp"], -85.);
    assert_eq!(value["carriers"].as_array().unwrap().len(), 2);
    assert_eq!(value["neighbors"].as_array().unwrap().len(), 2);
    let lte = value["neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["rat"] == "LTE")
        .unwrap();
    assert_eq!(lte["rsrp"], -97.);
    assert_eq!(lte["rsrq"], -12.);
    assert_eq!(value["neighborState"]["source"], "qmodem.send_at");
}
#[test]
fn unsupported_ca_does_not_disable_neighbors_and_unsupported_neighbors_do_not_disable_ca() {
    for failed in ["AT+QCAINFO", "AT+QENG=\"neighbourcell\""] {
        let value = qmodem_at::collect(
            &row(),
            |command| {
                if command == failed {
                    Err(PanelError::new("AT_UNSUPPORTED", 422))
                } else {
                    Ok(raw(command))
                }
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(value["partial"], true);
        if failed == "AT+QCAINFO" {
            assert_eq!(value["neighbors"].as_array().unwrap().len(), 2);
            assert_eq!(value["neighborState"]["status"], "reported");
            assert_eq!(value["carrierState"]["queryStatus"], "error");
        } else {
            assert_eq!(value["carriers"].as_array().unwrap().len(), 2);
            assert_eq!(value["neighborState"]["queryStatus"], "error");
        }
    }
}
#[test]
fn timeout_stops_the_round_and_changed_serving_never_publishes_unverified_inventory() {
    let mut calls = vec![];
    let partial = qmodem_at::collect(
        &row(),
        |command| {
            calls.push(command.to_owned());
            if command == "AT+QCAINFO" {
                Err(PanelError::new("SYSTEM_UNAVAILABLE", 503))
            } else {
                Ok(raw(command))
            }
        },
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(calls.len(), 2);
    assert!(partial["neighbors"].as_array().unwrap().is_empty());
    assert_eq!(partial["infoSupplementDeferred"], true);
    let mut primary_reads = 0;
    assert_eq!(
        qmodem_at::collect(
            &row(),
            |command| {
                if command == "AT+QENG=\"servingcell\"" {
                    primary_reads += 1;
                    if primary_reads == 2 {
                        return Ok(SERVING.replace(",214,", ",215,"));
                    }
                }
                Ok(raw(command))
            },
            |stage| {
                assert!(stage["neighbors"].as_array().unwrap().is_empty());
                Ok(())
            }
        )
        .unwrap_err()
        .code,
        "QMODEM_AT_SERVING_CHANGED"
    );
}
#[test]
fn plans_reuse_reviewed_independent_queries_and_never_enable_active_search_or_examples() {
    for (_, models) in wlmb_server::catalog::MODELS {
        for model in models.split_whitespace() {
            let mut device = row();
            device["model"] = json!(model);
            if let Some(plan) = qmodem_at::plan(&device) {
                let profile = wlmb_server::catalog::identify(model);
                assert!(plan.commands.len() <= 3);
                for command in plan.commands {
                    assert!(
                        profile["queries"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|v| v == &command)
                    );
                    assert!(qmodem_at::command_allowed(&command));
                    assert!(!command.contains("QSCAN") && !command.contains("CNWSEARCH"));
                }
            }
        }
    }
    for model in ["unknown", "gm800", "simcom-sim8200ea-m2"] {
        let mut device = row();
        device["model"] = json!(model);
        assert!(qmodem_at::plan(&device).is_none());
    }
    let mut device = row();
    device["platform"] = json!("unisoc");
    assert!(qmodem_at::plan(&device).is_none());
}

#[test]
fn partial_neighbor_metrics_remain_visible_and_unknown_optional_layout_is_not_empty_success() {
    let value=qmodem_at::collect(&row(),|command|Ok(if command=="AT+QENG=\"neighbourcell\"" {
        "+QENG: \"neighbourcell\",\"NR5G\",627264,30,-95,-13\n+QENG: \"neighbourcell intra\",\"LTE\",1650,40,-12,-97\nOK\n".into()
    }else{raw(command)}),|_|Ok(())).unwrap();
    assert_eq!(value["neighbors"].as_array().unwrap().len(), 2);
    for row in value["neighbors"].as_array().unwrap() {
        assert!(row["sinr"].is_null());
        assert!(!row["rsrp"].is_null());
    }
    let invalid = qmodem_at::collect(
        &row(),
        |command| {
            Ok(if command == "AT+QENG=\"neighbourcell\"" {
                "+QENG: \"neighbourcell\",\"NR5G\",bad,9999,-95,-13\nOK\n".into()
            } else {
                raw(command)
            })
        },
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(invalid["neighborState"]["queryStatus"], "error");
    assert_eq!(invalid["neighborState"]["code"], "AT_PARSE_UNAVAILABLE");
    assert_eq!(invalid["carriers"].as_array().unwrap().len(), 2);
}

#[test]
fn fibocom_native_raw_transport_uses_the_same_hex_decoding_as_standalone() {
    let mut device = row();
    device["model"] = json!("fm160-cn");
    let main = "+GTCCINFO:\n1,9,460,01,ABC,12345,992F0,1E8,5078,12,79,0,76,69\n2,4,460,01,ABC,23456,708,5B,0,0,51,22\nOK\n";
    let ca = "+GTCAINFO:\nPCC: 5078,1E8,992F0,12,0,0,0,0,0\nSCC 1: 2,0,5078,1E9,9B700,12,0,0,0,0,0,0\nOK\n";
    let value = qmodem_at::collect(
        &device,
        |command| Ok(if command == "AT+GTCCINFO?" { main } else { ca }.into()),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(value["serving"][0]["rsrp"], -81.);
    assert_eq!(value["neighbors"][0]["channel"], 1800.);
    assert_eq!(value["neighbors"][0]["pci"], 91.);
    assert_eq!(value["carriers"].as_array().unwrap().len(), 2);
}

#[test]
fn compact_and_extended_lte_neighbor_columns_remain_distinct() {
    let value=wlmb_server::radio::parse("quectel",
        "+QENG: \"neighbourcell\",\"LTE\",1650,40,-97,-12,3\n+QENG: \"neighbourcell\",\"LTE\",1650,41,-13,-98,-75,8\nOK\n").unwrap();
    let rows = value["neighbors"].as_array().unwrap();
    let compact = rows.iter().find(|row| row["pci"] == 40.).unwrap();
    assert_eq!(compact["rsrp"], -97.);
    assert!(compact["sinr"].is_null());
    let extended = rows.iter().find(|row| row["pci"] == 41.).unwrap();
    assert_eq!(extended["rsrp"], -98.);
    assert_eq!(extended["rsrq"], -13.);
    assert_eq!(extended["sinr"], 8.);
}

#[test]
fn selected_sms_tool_completion_is_distinct_from_truncation_or_empty_output() {
    let command = "AT+QENG=\"neighbourcell\"";
    let stripped = NEIGHBOR.replace("OK\r\n", "");
    assert!(qmodem_at::response(&reply(command, &stripped), &row(), command).is_err());
    let mut tool = row();
    tool["qmodemAtTool"] = json!("sms_tool_q");
    let normalized = qmodem_at::response(&reply(command, &stripped), &tool, command).unwrap();
    assert!(normalized.ends_with("OK\n"));
    assert_eq!(
        wlmb_server::radio::parse("quectel", &normalized).unwrap()["neighbors"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    for raw in ["", "\r\n", "AT+QENG=\"neighbourcell\"\r\n", SERVING] {
        assert!(qmodem_at::response(&reply(command, raw), &tool, command).is_err());
    }
    let mut failure = reply(command, &stripped);
    failure["at_cfg"]["status"] = json!("0");
    assert!(qmodem_at::response(&failure, &tool, command).is_err());
    let carrier_event = format!("{stripped}\nNO CARRIER\n");
    assert!(qmodem_at::response(&reply(command, &carrier_event), &tool, command).is_err());
}

#[test]
fn transport_failures_stop_queries_and_do_not_hide_behind_earlier_unsupported_error() {
    for failure in [
        "BUSY",
        "NO CARRIER",
        "NO DIALTONE",
        "+CME ERROR: 3",
        "+CMS ERROR: 500",
    ] {
        let mut calls = vec![];
        let value = qmodem_at::collect(
            &row(),
            |command| {
                calls.push(command.to_owned());
                let response = if command == "AT+QCAINFO" {
                    "ERROR\n"
                } else if command == "AT+QENG=\"neighbourcell\"" {
                    failure
                } else {
                    SERVING
                };
                qmodem_at::response(&reply(command, response), &row(), command)
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(value["carrierState"]["code"], "AT_FAILED");
        assert_eq!(value["neighborState"]["code"], "AT_FAILED");
        assert_eq!(value["infoSupplementDeferred"], true);
        assert!(value["neighbors"].as_array().unwrap().is_empty());
    }
}

#[test]
fn helper_normalization_cannot_exceed_response_size_limit() {
    let command = "AT+QENG=\"neighbourcell\"";
    let mut device = row();
    device["qmodemAtTool"] = json!("sms_tool_q");
    let mut response = NEIGHBOR.replace("OK\r\n", "");
    response.push_str(&" ".repeat(wlmb_server::MAX_AT_RESPONSE - response.len()));
    assert_eq!(
        qmodem_at::response(&reply(command, &response), &device, command)
            .unwrap_err()
            .code,
        "AT_RESPONSE_MISMATCH"
    );
}
