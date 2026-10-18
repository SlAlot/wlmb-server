#![cfg(unix)]
#![allow(clippy::unwrap_used)]
// 模拟旧版与新版 ubus 签名；只委托 QModem，不打开串口，不调用其邻区整理方法。
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use wlmb_server::{
    Result, qmodem_at,
    radio_controls::RadioControls,
    transport::{Broker, Bus, Discovery},
};
struct Inventory(Vec<Value>);
impl Discovery for Inventory {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(self.0.clone())
    }
    fn owners(&self, _: &str) -> Result<Vec<String>> {
        panic!("委托共享查询不得接管串口")
    }
}
struct RawBus {
    style: u64,
    open: bool,
    strip_ok: bool,
    calls: Mutex<Vec<Value>>,
}
impl Bus for RawBus {
    fn describe(&self, object: &str, _: Duration) -> Result<Value> {
        Ok(if object == "qmodem" {
            json!({"send_at":{"config_section":3,"params":self.style},
                "cell_info":{"config_section":3},"get_current_band":{"config_section":3},
                "get_neighborcell":{"config_section":3}})
        } else {
            json!({})
        })
    }
    fn call(&self, object: &str, method: &str, params: Value, _: Duration) -> Result<Value> {
        if object == "at-daemon" && method == "list" {
            return Ok(if self.open {
                json!({"ports":[{"port":"/dev/ttyUSB2","is_open":1}]})
            } else {
                json!({"ports":[]})
            });
        }
        assert_eq!(object, "qmodem");
        assert_eq!(
            method, "send_at",
            "不得使用上游整理后的邻区、载波或任意写入方法"
        );
        assert!(qmodem_at::request_valid(&params));
        assert_eq!(params["params"].is_string(), self.style == 3);
        self.calls.lock().unwrap().push(params.clone());
        let payload = if self.style == 3 {
            serde_json::from_str::<Value>(params["params"].as_str().unwrap()).unwrap()
        } else {
            params["params"].clone()
        };
        let command = payload["at"].as_str().unwrap();
        let raw = match command {
            "AT+QENG=\"servingcell\"" => {
                "+QENG: \"servingcell\",\"NOCONN\",\"NR5G-SA\",\"TDD\",460,01,1234AB,214,ABCD,627264,78,12,-85,-10,19,1,-\r\nOK\r\n"
            }
            "AT+QCAINFO" => {
                "+QCAINFO: \"PCC\",627264,100,\"NR5G BAND 78\",1,214\r\n+QCAINFO: \"SCC\",636664,80,\"NR5G BAND 78\",2,215\r\nOK\r\n"
            }
            "AT+QENG=\"neighbourcell\"" => {
                "+QENG: \"neighbourcell\",\"NR5G\",627264,30,-95,-13,5\r\nOK\r\n"
            }
            _ => panic!("不允许额外的 AT"),
        };
        let raw = if self.strip_ok {
            raw.replace("OK\r\n", "")
        } else {
            raw.to_owned()
        };
        Ok(
            json!({"at_cfg":{"status":"1","cmd":format!("at /dev/ttyUSB2 {command}"),"res":raw},
            "ignored":"synthetic-private-field"}),
        )
    }
}
fn inventory(use_ubus: Option<&str>, at_tool: &str) -> Arc<Inventory> {
    let flag = use_ubus
        .map(|v| format!("qmodem.2_1.use_ubus='{v}'\n"))
        .unwrap_or_default();
    let mut rows = wlmb_server::transport::parse_config(&format!(
        "qmodem.main=main\nqmodem.main.at_tool='{at_tool}'\nqmodem.2_1=modem\nqmodem.2_1.at_port='/dev/ttyUSB2'\nqmodem.2_1.name='rm520f-gl'\n{flag}"
    ));
    rows[0]["instance"] = json!("test-instance");
    Arc::new(Inventory(rows))
}
#[test]
fn native_at_is_automatic_read_only_and_uses_configured_qmodem_flags_for_both_abis() {
    for style in [2, 3] {
        for flag in [None, Some("0"), Some("1")] {
            for tool in ["0", "1"] {
                let bus = Arc::new(RawBus {
                    style,
                    open: false,
                    strip_ok: tool == "1",
                    calls: Mutex::new(vec![]),
                });
                let broker = Broker::new(bus.clone(), inventory(flag, tool));
                let row = broker.modems().unwrap().remove(0);
                assert_eq!(row["infoInventorySource"], "qmodem.send_at");
                assert_eq!(row["infoAtParams"], style);
                assert_eq!(row["infoReadOnly"], true);
                assert_eq!(row["infoNeighborQuery"], true);
                assert!(!RadioControls::supported(&row));
                assert!(broker.query_bound(&row, "AT+CFUN=0", 3).is_err());
                let value = broker.info_snapshot(&row).unwrap().unwrap();
                assert_eq!(value["neighbors"][0]["sinr"], 5.);
                assert_eq!(value["carriers"].as_array().unwrap().len(), 2);
                assert!(!value.to_string().contains("synthetic-private-field"));
                assert_eq!(bus.calls.lock().unwrap().len(), 4);
                for request in bus.calls.lock().unwrap().iter() {
                    let params = if style == 3 {
                        serde_json::from_str::<Value>(request["params"].as_str().unwrap()).unwrap()
                    } else {
                        request["params"].clone()
                    };
                    assert_eq!(
                        params["use_ubus_flag"],
                        if flag == Some("0") { "0" } else { "1" }
                    );
                    assert_eq!(params["port"], "/dev/ttyUSB2");
                }
            }
        }
    }
}
#[test]
fn ready_at_daemon_remains_preferred_and_unknown_abi_does_not_guess_or_open_ports() {
    let bus = Arc::new(RawBus {
        style: 3,
        open: true,
        strip_ok: false,
        calls: Mutex::new(vec![]),
    });
    let broker = Broker::new(bus.clone(), inventory(None, "0"));
    let row = broker.modems().unwrap().remove(0);
    assert_eq!(row["shared"], true);
    assert!(row["infoAtParams"].is_null());
    assert!(broker.info_snapshot(&row).unwrap().is_none());
    assert!(bus.calls.lock().unwrap().is_empty());
    let bus = Arc::new(RawBus {
        style: 8,
        open: false,
        strip_ok: false,
        calls: Mutex::new(vec![]),
    });
    let broker = Broker::new(bus.clone(), inventory(None, "0"));
    assert!(broker.modems().unwrap()[0]["infoAtParams"].is_null());
    assert!(bus.calls.lock().unwrap().is_empty());
}
