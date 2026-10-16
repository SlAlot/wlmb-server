#![cfg(unix)]
#![allow(clippy::unwrap_used)]
// 使用 Mudi7 回复结构及合成短信回归；不访问真实串口，不把样本当全系列实机验收。
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use wlmb_server::{
    PanelError, Result,
    glinet::Glinet,
    sampler::Source,
    sms_runtime::{Factory, ForwardRuntime, Resources, Settings, SmsDevice, SmsHub, Store},
    transport::Bus,
};
struct Fixture {
    model: String,
    objects: Mutex<Vec<Value>>,
    commands: Mutex<Vec<String>>,
    stores: Mutex<usize>,
    reply: Mutex<Option<Value>>,
    changed: bool,
    mode: u8,
    empty: bool,
    restart: bool,
    offline_object: Option<String>,
}
fn object(name: &str, id: u32) -> Value {
    json!({"name":name,"objectId":id,"signature":{"get_result_AT":{"cmd":3,"timeout":5,"source_flag":5,"sub_id":5}}})
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            model: "RG650V-EU".into(),
            objects: Mutex::new(vec![object("modem.CPU.AT", 1234)]),
            commands: Mutex::new(vec![]),
            stores: Mutex::new(0),
            reply: Mutex::new(None),
            changed: false,
            mode: 0,
            empty: true,
            restart: false,
            offline_object: None,
        }
    }
}
impl Bus for Fixture {
    fn objects(&self, pattern: &str, timeout: Duration) -> Result<Vec<Value>> {
        assert_eq!(pattern, "modem.*.AT");
        assert!(timeout <= Duration::from_secs(3));
        Ok(self.objects.lock().unwrap().clone())
    }
    fn call(&self, object: &str, method: &str, args: Value, timeout: Duration) -> Result<Value> {
        assert!(wlmb_server::glinet::object_name(object));
        assert_eq!(method, "get_result_AT");
        assert_eq!(args.as_object().unwrap().len(), 2);
        assert_eq!(
            timeout,
            Duration::from_secs(args["timeout"].as_u64().unwrap() + 2)
        );
        let cmd = args["cmd"].as_str().unwrap();
        self.commands.lock().unwrap().push(cmd.into());
        if self.offline_object.as_deref() == Some(object) {
            return Err(PanelError::new("AT_FAILED", 409));
        }
        if let Some(value) = self.reply.lock().unwrap().clone() {
            return Ok(value);
        }
        let data = match cmd {
            "AT+CGMM" => format!("\r\n{}\r\n\r\nOK\r\n", self.model),
            "AT+CSQ" => "+CSQ: 20,99\r\nOK\r\n".into(),
            "AT+CMGF?" => format!("+CMGF: {}\r\nOK\r\n", self.mode),
            "AT+CPMS?" => {
                let mut n = self.stores.lock().unwrap();
                *n += 1;
                format!(
                    "+CPMS: \"{}\",{},127,\"ME\",{},127,\"ME\",{},127\r\nOK\r\n",
                    if self.changed && *n > 1 { "SM" } else { "ME" },
                    usize::from(!self.empty),
                    usize::from(!self.empty),
                    usize::from(!self.empty)
                )
            }
            "AT+CMGL=4" => {
                if self.restart {
                    self.objects.lock().unwrap()[0]["objectId"] = json!(2222);
                }
                if self.empty {
                    "OK\r\n".into()
                } else {
                    "+CMGL: 1,0,,18\r\n000005810080F6000862010121436523024F60\r\nOK\r\n".into()
                }
            }
            _ => return Err(PanelError::new("AT_UNSUPPORTED", 422)),
        };
        Ok(json!({"data":data,"channel_status":true}))
    }
}
#[test]
fn discovers_cpu_and_usb_objects_without_assuming_cpu_or_a_port() {
    let bus = Arc::new(Fixture::default());
    *bus.objects.lock().unwrap() = vec![
        object("modem.USB_1-1.3.AT", 1235),
        object("modem.CPU.AT", 1234),
    ];
    let gl = Glinet::new(bus.clone());
    let rows = gl.modems().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r["model"] == "rg650v-eu"
        && r["shared"] == true
        && r["identityVerified"] == true));
    assert_eq!(gl.modems().unwrap(), rows);
    assert_eq!(
        bus.commands.lock().unwrap().len(),
        2,
        "常规展示不重复查询模组身份或短信"
    );
    assert!(!gl.writable());
    assert!(!gl.notice_supported());
    assert_eq!(
        gl.query_bound(&rows[0], "AT+CSQ").unwrap(),
        "+CSQ: 20,99\r\nOK\r\n"
    );
    assert_eq!(
        gl.query_bound(&rows[0], "AT+CFUN=1,1").unwrap_err().code,
        "AT_WRITE_DISABLED"
    );
    assert_eq!(
        gl.query_bound(&rows[0], "AT+CMGS=20").unwrap_err().code,
        "AT_WRITE_DISABLED"
    );
    assert_eq!(
        gl.send(rows[0]["id"].as_str().unwrap(), "10086", "test")
            .unwrap_err()
            .code,
        "SMS_SHARED_READ_ONLY"
    );
    assert!(
        !bus.commands
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.starts_with("AT+CMG") || c.starts_with("AT+CFUN"))
    );
}
#[test]
fn an_offline_usb_manager_does_not_hide_a_working_cpu_modem() {
    let bus = Arc::new(Fixture {
        objects: Mutex::new(vec![
            object("modem.A_USB.AT", 1235),
            object("modem.CPU.AT", 1234),
        ]),
        offline_object: Some("modem.A_USB.AT".into()),
        ..Fixture::default()
    });
    let gl = Glinet::new(bus.clone());
    let rows = gl.modems().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["object"], "modem.CPU.AT");
    assert_eq!(gl.modems().unwrap(), rows);
    assert_eq!(bus.commands.lock().unwrap().len(), 2);
}
struct Disabled;
impl Store for Disabled {
    fn load(&self) -> Result<Settings> {
        Ok(Settings::default())
    }
    fn save(&self, _: &Settings) -> Result<()> {
        Ok(())
    }
}
impl Factory for Disabled {
    fn create(&self, _: &str) -> Result<Resources> {
        panic!("短信收取和转发关闭时不得创建推送资源")
    }
}
#[test]
fn disabled_receive_and_forward_never_poll_glinet_or_allocate_push_resources() {
    let bus = Arc::new(Fixture::default());
    let gl = Glinet::new(bus.clone());
    let runtime =
        ForwardRuntime::new(SmsHub::new(gl), Arc::new(Disabled), Arc::new(Disabled)).unwrap();
    runtime.start_saved().unwrap();
    assert_eq!(runtime.public().unwrap()["runtime"]["running"], false);
    assert!(bus.commands.lock().unwrap().is_empty());
    runtime.shutdown();
}
#[test]
fn shared_transport_uses_every_catalog_identity_instead_of_a_mudi_only_model_list() {
    for (_, models) in wlmb_server::catalog::MODELS {
        for model in models.split_whitespace() {
            let bus = Arc::new(Fixture {
                model: model.into(),
                ..Fixture::default()
            });
            let gl = Glinet::new(bus);
            let row = gl.modems().unwrap().remove(0);
            assert_eq!(row["model"], model);
            assert_eq!(row["profile"], wlmb_server::catalog::identify(model));
            assert!(!wlmb_server::radio_controls::RadioControls::supported(&row));
        }
    }
    let bus = Arc::new(Fixture {
        model: "UNREVIEWED-MODEL".into(),
        ..Fixture::default()
    });
    let gl = Glinet::new(bus);
    let row = gl.modems().unwrap().remove(0);
    assert_eq!(row["profile"]["family"], "generic");
    assert_eq!(row["smsReadSupported"], false);
    assert!(gl.read(row["id"].as_str().unwrap()).is_err());
}
#[test]
fn objects_and_requests_are_bounded_and_do_not_allow_mutating_methods() {
    for name in ["modem.CPU.AT", "modem.1-1.3.AT", "modem.USB0.AT"] {
        assert!(wlmb_server::glinet::object_name(name));
    }
    for name in [
        "modem..AT",
        "modem.x/../../.AT",
        "modem.*.AT",
        "modem.x\n.AT",
    ] {
        assert!(!wlmb_server::glinet::object_name(name));
    }
    assert!(wlmb_server::glinet::request_valid(
        &json!({"cmd":"AT+CMGF?","timeout":5})
    ));
    for args in [
        json!({"cmd":"AT\nAT+CFUN=0","timeout":5}),
        json!({"cmd":"AT","timeout":0}),
        json!({"cmd":"AT","timeout":11}),
        json!({"cmd":"AT","timeout":5,"sub_id":0}),
    ] {
        assert!(!wlmb_server::glinet::request_valid(&args));
    }
    let bus = Arc::new(Fixture::default());
    bus.objects.lock().unwrap()[0]["signature"]["get_result_AT"]["timeout"] = json!(3);
    assert!(Glinet::new(bus.clone()).modems().unwrap().is_empty());
    assert!(bus.commands.lock().unwrap().is_empty());
    bus.objects
        .lock()
        .unwrap()
        .push(object("modem.CPU.AT", 9000));
    assert!(Glinet::new(bus.clone()).modems().is_err());
    assert!(bus.commands.lock().unwrap().is_empty());
}
#[test]
fn reply_errors_and_service_restarts_fail_closed_without_replaying_commands() {
    for reply in [
        json!({"data":"OK\r\n","channel_status":false}),
        json!({"data":"+CSQ: 20,99\r\n","channel_status":true}),
        json!({"data":"OK\r\nOK\r\n","channel_status":true}),
        json!({"data":"ERROR\r\n","channel_status":true}),
    ] {
        let bus = Arc::new(Fixture::default());
        let gl = Glinet::new(bus.clone());
        let row = gl.modems().unwrap().remove(0);
        *bus.reply.lock().unwrap() = Some(reply);
        assert!(gl.query_bound(&row, "AT+CSQ").is_err());
        assert_eq!(
            bus.commands
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.as_str() == "AT+CSQ")
                .count(),
            1
        );
    }
    let bus = Arc::new(Fixture::default());
    let gl = Glinet::new(bus.clone());
    let row = gl.modems().unwrap().remove(0);
    bus.objects.lock().unwrap()[0]["objectId"] = json!(4321);
    assert!(gl.query_bound(&row, "AT+CSQ").is_err());
    assert!(!bus.commands.lock().unwrap().contains(&"AT+CSQ".into()));
}
#[test]
fn pdu_empty_and_real_message_share_the_standard_read_state_cache() {
    for empty in [true, false] {
        let bus = Arc::new(Fixture {
            empty,
            ..Fixture::default()
        });
        let gl = Glinet::new(bus.clone());
        let key = gl.modems().unwrap()[0]["id"].as_str().unwrap().to_owned();
        let hub = SmsHub::new(gl.clone());
        let records = hub.refresh(&key).unwrap();
        assert_eq!(records.len(), usize::from(!empty));
        if !empty {
            assert_eq!(records[0].message.content, "你");
        }
        assert_eq!(hub.refresh(&key).unwrap().len(), records.len());
        assert_eq!(
            bus.commands
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.as_str() == "AT+CMGL=4")
                .count(),
            1
        );
        assert!(!bus.commands.lock().unwrap().iter().any(|c| {
            ["AT+CNMI=", "AT+CMGF=", "AT+CPMS=", "AT+CSCS="]
                .iter()
                .any(|p| c.starts_with(p))
        }));
        assert_eq!(gl.source_info(&key)["eventConnected"], false);
    }
}
#[test]
fn storage_changes_text_mode_and_restart_during_a_scan_never_publish_messages() {
    for bus in [
        Fixture {
            changed: true,
            ..Fixture::default()
        },
        Fixture {
            mode: 1,
            ..Fixture::default()
        },
        Fixture {
            restart: true,
            ..Fixture::default()
        },
    ] {
        let bus = Arc::new(bus);
        let gl = Glinet::new(bus.clone());
        let key = gl.modems().unwrap()[0]["id"].as_str().unwrap().to_owned();
        assert!(gl.read(&key).is_err());
        assert!(
            bus.commands
                .lock()
                .unwrap()
                .iter()
                .all(|c| !c.contains("=0") && !c.contains("=1"))
        );
    }
}
