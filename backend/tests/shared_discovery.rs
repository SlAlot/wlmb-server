#![cfg(unix)]
#![allow(clippy::unwrap_used)]
// 参考QModem省略默认配置的固件变体，只通过服务列举和端口归属协商共享通道。
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use wlmb_server::{
    PanelError, Result, catalog,
    transport::{self, Broker, Bus, Discovery},
};

struct Inventory {
    rows: Mutex<Vec<Value>>,
    owners: Result<Vec<String>>,
}
impl Discovery for Inventory {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(self.rows.lock().unwrap().clone())
    }
    fn owners(&self, _: &str) -> Result<Vec<String>> {
        self.owners.clone()
    }
}
struct Daemon {
    ports: Mutex<Result<Value>>,
    commands: Mutex<Vec<String>>,
}
impl Bus for Daemon {
    fn call(&self, object: &str, method: &str, args: Value, _: Duration) -> Result<Value> {
        assert_eq!(object, "at-daemon");
        match method {
            "list" => self.ports.lock().unwrap().clone(),
            "sendat" => {
                self.commands
                    .lock()
                    .unwrap()
                    .push(args["at_cmd"].as_str().unwrap().into());
                Ok(json!({"status":"success","response":"OK\r\n"}))
            }
            _ => panic!("不能打开、关闭或接管已有端口"),
        }
    }
}
fn config(model: &str, flags: &str) -> Vec<Value> {
    let raw = format!(
        "qmodem.main=main\nqmodem.m1=modem\nqmodem.m1.name='{model}'\nqmodem.m1.at_port='/dev/ttyUSB1'\n{flags}"
    );
    let mut rows = transport::parse_config(&raw);
    for row in &mut rows {
        row["instance"] = json!("48129:63");
    }
    rows
}
fn setup(
    rows: Vec<Value>,
    ports: Result<Value>,
    owners: Result<Vec<String>>,
) -> (Broker, Arc<Daemon>, Arc<Inventory>) {
    let bus = Arc::new(Daemon {
        ports: Mutex::new(ports),
        commands: Mutex::new(vec![]),
    });
    let inventory = Arc::new(Inventory {
        rows: Mutex::new(rows),
        owners,
    });
    (Broker::new(bus.clone(), inventory.clone()), bus, inventory)
}
fn open() -> Result<Value> {
    Ok(json!({"ports":[{"port":"/dev/ttyUSB1","is_open":1}]}))
}
fn owner() -> Result<Vec<String>> {
    Ok(vec!["ubus-at-daemon".into()])
}

#[test]
fn omitted_defaults_work_for_every_catalog_model_without_a_model_special_case() {
    let mut count = 0;
    for (_, models) in catalog::MODELS {
        for model in models.split_whitespace() {
            let rows = config(model, "");
            assert_eq!(rows[0]["shared"], false);
            assert_eq!(rows[0]["sharedPolicy"], "automatic");
            let (broker, bus, _) = setup(rows, open(), owner());
            let row = broker.modems().unwrap().remove(0);
            assert_eq!(row["shared"], true, "{model}");
            assert_eq!(row["sharedDetectedBy"], "open-daemon-port");
            assert!(broker.query_bound(&row, "AT", 3).is_ok(), "{model}");
            assert_eq!(*bus.commands.lock().unwrap(), ["AT"]);
            count += 1;
        }
    }
    assert!(count > 0);
}

#[test]
fn explicit_disable_or_alternative_tool_is_not_overridden_by_detection() {
    for flags in [
        "qmodem.m1.use_ubus='0'\n",
        "qmodem.main.at_tool='1'\n",
        "qmodem.m1.use_ubus='invalid'\n",
    ] {
        let (broker, bus, _) = setup(config("rm520n-gl", flags), open(), owner());
        assert_eq!(broker.modems().unwrap()[0]["shared"], false);
        assert_eq!(
            broker.query("m1", "AT", None).unwrap_err().code,
            "AT_NOT_SHARED"
        );
        assert!(bus.commands.lock().unwrap().is_empty());
    }
}

#[test]
fn unavailable_ambiguous_closed_and_conflicting_ports_never_become_shared() {
    for (ports, owners) in [
        (Err(PanelError::new("UBUS_OBJECT_NOT_FOUND", 503)), owner()),
        (Ok(json!({"ports":[]})), owner()),
        (
            Ok(json!({"ports":[{"port":"/dev/ttyUSB1","is_open":0}]})),
            owner(),
        ),
        (
            Ok(
                json!({"ports":[{"port":"/dev/ttyUSB1","is_open":1},{"port":"/dev/ttyUSB1","is_open":1}]}),
            ),
            owner(),
        ),
        (open(), Ok(vec![])),
        (
            open(),
            Ok(vec!["ubus-at-daemon".into(), "other-plugin".into()]),
        ),
        (open(), Err(PanelError::new("AT_OWNERSHIP_UNKNOWN", 409))),
    ] {
        let (broker, bus, _) = setup(config("fm160-cn", ""), ports, owners);
        assert_eq!(broker.modems().unwrap()[0]["shared"], false);
        assert!(broker.query("m1", "AT", None).is_err());
        assert!(bus.commands.lock().unwrap().is_empty());
    }
    let mut rows = config("mt5700m-cn", "");
    let mut duplicate = rows[0].clone();
    duplicate["id"] = json!("m2");
    rows.push(duplicate);
    let (broker, bus, _) = setup(rows, open(), owner());
    assert!(
        broker
            .modems()
            .unwrap()
            .iter()
            .all(|r| r["shared"] == false)
    );
    assert!(bus.commands.lock().unwrap().is_empty());
}

#[test]
fn port_closure_and_config_changes_are_rechecked_before_dispatch() {
    let (broker, bus, inventory) = setup(config("mt5700m-cn", ""), open(), owner());
    let row = broker.modems().unwrap().remove(0);
    *bus.ports.lock().unwrap() = Ok(json!({"ports":[]}));
    assert!(broker.query_bound(&row, "AT", 3).is_err());
    *bus.ports.lock().unwrap() = open();
    *inventory.rows.lock().unwrap() = config("mt5700m-cn", "qmodem.m1.use_ubus='0'\n");
    assert!(broker.query_bound(&row, "AT", 3).is_err());
    assert!(bus.commands.lock().unwrap().is_empty());
}
