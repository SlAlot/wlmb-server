#![cfg(unix)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
#![allow(clippy::unwrap_used)]
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use wlmb_server::{
    Result,
    compat_sms::CompatSms,
    sms_runtime::SmsDevice,
    transport::{Broker, Bus, Discovery},
};
struct Device {
    commands: Mutex<Vec<String>>,
    mode: &'static str,
    changed: bool,
    stores: Mutex<u32>,
    instance: Arc<Mutex<&'static str>>,
    change_on_list: bool,
    empty: bool,
    transient_storage: bool,
}
impl Bus for Device {
    fn call(&self, object: &str, method: &str, args: Value, timeout: Duration) -> Result<Value> {
        assert_eq!(object, "at-daemon");
        if method == "list" {
            return Ok(json!({"ports":[{"port":"/dev/ttyUSB1","is_open":1}]}));
        }
        assert_eq!(method, "sendat");
        let cmd = args["at_cmd"].as_str().unwrap();
        self.commands.lock().unwrap().push(cmd.into());
        let response = match cmd {
            "AT+CMGF?" => self.mode.to_owned(),
            "AT+CPMS?" => {
                let mut n = self.stores.lock().unwrap();
                *n += 1;
                if self.transient_storage && *n == 1 {
                    return Ok(json!({"status":"success","response":"OK\r\n"}));
                }
                format!(
                    "+CPMS: \"{}\",1,100,\"ME\",1,100,\"ME\",1,100\r\nOK\r\n",
                    if self.changed && *n > 1 { "SM" } else { "ME" }
                )
            }
            "AT+CMGL=4" => {
                assert_eq!(args["timeout"], 10);
                assert_eq!(timeout, Duration::from_secs(12));
                if self.empty {
                    "OK\r\n".into()
                } else {
                    "+CMGL: 1,0,,18\r\n000005810080F6000862010121436523024F60\r\nOK\r\n".into()
                }
            }
            "AT+CSCS?" => "+CSCS: \"UCS2\"\nOK".into(),
            "AT+CSDH?" => "+CSDH: 1\nOK".into(),
            "AT+CMGL=\"ALL\"" => {
                assert_eq!(args["timeout"], 10);
                if self.change_on_list {
                    *self.instance.lock().unwrap() = "1:2";
                }
                "+CMGL: 1,\"REC UNREAD\",\"00310030003000380036\",\"\",\"26/10/11,12:34:56+32\",129,4\n4F60\nOK".into()
            }
            "AT+CMGR=1" => {
                assert_eq!(args["timeout"], 1);
                assert_eq!(timeout, Duration::from_secs(3));
                "+CMGR: \"REC READ\",\"00310030003000380036\",\"\",\"26/10/11,12:34:56+32\",129,0,0,8,\"0031\",129,4\n4F60\nOK".into()
            }
            _ => panic!("unexpected command"),
        };
        Ok(json!({"status":"success","response":response}))
    }
}
struct Config {
    owner: &'static str,
    instance: Arc<Mutex<&'static str>>,
}
impl Discovery for Config {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![
            json!({"id":"m1","model":"mt5700m-cn","port":"/dev/ttyUSB1","shared":true,"instance":*self.instance.lock().unwrap()}),
        ])
    }
    fn owners(&self, _: &str) -> Result<Vec<String>> {
        Ok(vec![self.owner.into()])
    }
}
fn setup(mode: &'static str, changed: bool, owner: &'static str) -> (CompatSms, Arc<Device>) {
    setup_race(mode, changed, owner, false)
}
fn setup_race(
    mode: &'static str,
    changed: bool,
    owner: &'static str,
    change_on_list: bool,
) -> (CompatSms, Arc<Device>) {
    let instance = Arc::new(Mutex::new("1:1"));
    let bus = Arc::new(Device {
        commands: Mutex::new(vec![]),
        mode,
        changed,
        stores: Mutex::new(0),
        instance: instance.clone(),
        change_on_list,
        empty: false,
        transient_storage: false,
    });
    (
        CompatSms::new(Arc::new(Broker::new(
            bus.clone(),
            Arc::new(Config { owner, instance }),
        ))),
        bus,
    )
}
#[test]
fn shared_text_reads_with_bounded_metadata_timeout_and_rejects_hotplug_response() {
    let (sms, bus) = setup("+CMGF:1\nOK", false, "ubus-at-daemon");
    let rows = sms.read("m1").unwrap();
    assert_eq!(rows[0].message.content, "你");
    assert_eq!(rows[0].dcs, 8);
    assert!(!sms.writable());
    assert!(
        !bus.commands
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("AT+CMGF=") || s.starts_with("AT+CSCS="))
    );
    let (sms, bus) = setup_race("+CMGF:1\nOK", false, "ubus-at-daemon", true);
    assert_eq!(sms.read("m1").err().unwrap().code, "MODEM_CHANGED");
    assert!(
        !bus.commands
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("AT+CMGR="))
    );
}
#[test]
fn shared_pdu_standard_list_uses_plugin_read_state_and_never_sends_or_deletes() {
    let (sms, bus) = setup("+CMGF:0\r\nOK\r\n", false, "ubus-at-daemon");
    assert!(!sms.writable());
    let rows = sms.read("m1").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].message.content, "你");
    assert_eq!(
        *bus.commands.lock().unwrap(),
        vec!["AT+CMGF?", "AT+CPMS?", "AT+CMGL=4", "AT+CPMS?", "AT+CMGF?"]
    );
    assert!(sms.send("m1", "10086", "test").is_err());
    assert_eq!(bus.commands.lock().unwrap().len(), 5);
}
#[test]
fn incompatible_mode_and_owner_fail_without_mutation_or_fallback() {
    for mode in ["+CMGF: 2\r\nOK\r\n", "+CMGF:0\r\n+CMGF:1\r\nOK\r\n"] {
        let (sms, bus) = setup(mode, false, "ubus-at-daemon");
        assert_eq!(sms.read("m1").err().unwrap().code, "SMS_PDU_MODE_REQUIRED");
        assert_eq!(*bus.commands.lock().unwrap(), vec!["AT+CMGF?"]);
    }
    let (sms, bus) = setup("+CMGF:0\r\nOK\r\n", false, "other-owner");
    assert!(sms.read("m1").is_err());
    assert!(bus.commands.lock().unwrap().is_empty());
}
#[test]
fn storage_race_rejects_results_instead_of_mislabeling_messages() {
    let (sms, _) = setup("+CMGF:0\r\nOK\r\n", true, "ubus-at-daemon");
    assert_eq!(sms.read("m1").err().unwrap().code, "SMS_STORAGE_CHANGED");
}

#[test]
fn shared_empty_inbox_recovers_storage_metadata_without_replaying_listing() {
    let instance = Arc::new(Mutex::new("1:1"));
    let bus = Arc::new(Device {
        commands: Mutex::new(vec![]),
        mode: "+CMGF:0\r\nOK\r\n",
        changed: false,
        stores: Mutex::new(0),
        instance: instance.clone(),
        change_on_list: false,
        empty: true,
        transient_storage: true,
    });
    let sms = CompatSms::new(Arc::new(Broker::new(
        bus.clone(),
        Arc::new(Config {
            owner: "ubus-at-daemon",
            instance,
        }),
    )));
    assert!(sms.read("m1").unwrap().is_empty());
    let commands = bus.commands.lock().unwrap();
    assert_eq!(commands.iter().filter(|cmd| *cmd == "AT+CMGL=4").count(), 1);
    assert_eq!(commands.iter().filter(|cmd| *cmd == "AT+CPMS?").count(), 3);
    assert!(
        commands
            .iter()
            .all(|cmd| ["AT+CMGF?", "AT+CPMS?", "AT+CMGL=4"].contains(&cmd.as_str()))
    );
}
