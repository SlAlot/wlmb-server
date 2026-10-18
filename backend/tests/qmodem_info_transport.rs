#![cfg(unix)]
#![allow(clippy::unwrap_used)]
// 共享通道回归只使用模拟 ubus；禁止打开串口或调用任意 AT 接口。
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use wlmb_server::{
    PanelError, Result,
    radio_controls::RadioControls,
    transport::{Broker, Bus, Discovery},
};

struct Inventory(Mutex<Vec<Value>>);
impl Discovery for Inventory {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn owners(&self, _: &str) -> Result<Vec<String>> {
        panic!("原生信息接口不检查或接管串口")
    }
}
struct Native {
    calls: Mutex<Vec<String>>,
    open: bool,
    fail_network: bool,
    change_on_cell: Option<Arc<Inventory>>,
}
impl Bus for Native {
    fn describe(&self, object: &str, _: Duration) -> Result<Value> {
        Ok(if object == "qmodem" {
            json!({"cell_info":{"config_section":3},"network_info":{"config_section":3},"base_info":{"config_section":3}})
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
        assert_eq!(params, json!({"config_section":"2_1"}));
        self.calls.lock().unwrap().push(method.into());
        match method {
            "cell_info" => {
                if let Some(inventory) = &self.change_on_cell {
                    inventory.0.lock().unwrap()[0]["instance"] = json!("new-instance");
                }
                Ok(
                    json!({"modem_info":[{"key":"network_mode","value":"NR5G-SA Mode"},
                    {"key":"ARFCN","value":"627264"},{"key":"Physical Cell ID","value":"214"},
                    {"key":"RSRP","value":"-85"}]}),
                )
            }
            "network_info" if self.fail_network => Err(PanelError::new("SYSTEM_UNAVAILABLE", 503)),
            "network_info" => Ok(
                json!({"modem_info":[{"key":"AMBR DL","value":"3000"},{"key":"AMBR UL","value":"200"}]}),
            ),
            "base_info" => Ok(json!({"modem_info":[{"key":"temperature","value":"47°C"}]})),
            _ => panic!("不能调用写入或任意 AT 接口"),
        }
    }
}
fn inventory(flags: &str) -> Arc<Inventory> {
    let mut rows = wlmb_server::transport::parse_config(&format!(
        "qmodem.main=main\nqmodem.2_1=modem\nqmodem.2_1.at_port='/dev/ttyUSB2'\nqmodem.2_1.name='rm520f-gl'\n{flags}"
    ));
    rows[0]["instance"] = json!("test-instance");
    Arc::new(Inventory(Mutex::new(rows)))
}
#[test]
fn closed_daemon_uses_native_info_for_both_omitted_and_explicit_flags_without_write_permissions() {
    for flags in [
        "",
        "qmodem.2_1.use_ubus='1'\n",
        "qmodem.2_1.use_ubus='0'\n",
        "qmodem.main.at_tool='1'\n",
    ] {
        let bus = Arc::new(Native {
            calls: Mutex::new(vec![]),
            open: false,
            fail_network: false,
            change_on_cell: None,
        });
        let broker = Broker::new(bus.clone(), inventory(flags));
        let row = broker.modems().unwrap().remove(0);
        assert_eq!(row["infoTransport"], "qmodem-rpc");
        assert_eq!(row["shared"], false);
        assert!(!RadioControls::supported(&row));
        assert!(broker.query_bound(&row, "AT+CSQ", 3).is_err());
        let first = broker.info_snapshot(&row).unwrap().unwrap();
        assert_eq!(first["serving"][0]["rsrp"], -85.);
        assert_eq!(first["temperature"]["celsius"], 47.);
        assert_eq!(first["subscription"]["dlKbps"], 3000000.);
        assert_eq!(first["partial"], false);
        assert!(broker.info_snapshot(&row).unwrap().is_some());
        assert_eq!(
            bus.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|method| *method == "base_info")
                .count(),
            1
        );
    }
}
#[test]
fn ready_daemon_is_preferred_and_native_optional_errors_do_not_erase_signal() {
    let bus = Arc::new(Native {
        calls: Mutex::new(vec![]),
        open: true,
        fail_network: false,
        change_on_cell: None,
    });
    let broker = Broker::new(bus.clone(), inventory(""));
    let row = broker.modems().unwrap().remove(0);
    assert_eq!(row["shared"], true);
    assert!(row["infoTransport"].is_null());
    assert!(broker.info_snapshot(&row).unwrap().is_none());
    assert!(bus.calls.lock().unwrap().is_empty());
    let bus = Arc::new(Native {
        calls: Mutex::new(vec![]),
        open: false,
        fail_network: true,
        change_on_cell: None,
    });
    let broker = Broker::new(bus, inventory(""));
    let row = broker.modems().unwrap().remove(0);
    let value = broker.info_snapshot(&row).unwrap().unwrap();
    assert_eq!(value["serving"][0]["rsrp"], -85.);
    assert_eq!(value["partial"], true);
    assert_eq!(value["errors"][0]["code"], "SYSTEM_UNAVAILABLE");
}
#[test]
fn configuration_changes_and_duplicate_ports_cannot_publish_wrong_modem_data() {
    let inventory = inventory("");
    let bus = Arc::new(Native {
        calls: Mutex::new(vec![]),
        open: false,
        fail_network: false,
        change_on_cell: Some(inventory.clone()),
    });
    let broker = Broker::new(bus, inventory.clone());
    let row = broker.modems().unwrap().remove(0);
    assert_eq!(
        broker.info_snapshot(&row).unwrap_err().code,
        "MODEM_CHANGED"
    );
    let mut duplicate = inventory.0.lock().unwrap()[0].clone();
    duplicate["id"] = json!("2_2");
    inventory.0.lock().unwrap().push(duplicate);
    assert!(
        broker
            .modems()
            .unwrap()
            .iter()
            .all(|row| row["infoTransport"].is_null())
    );
}
