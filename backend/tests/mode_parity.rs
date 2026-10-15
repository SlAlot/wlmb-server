#![cfg(unix)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
#![allow(clippy::unwrap_used)]
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use wlmb_server::{
    PanelError, Result, catalog, controls,
    independent_at::{Endpoint, Hardware, IndependentSource},
    radio,
    radio_controls::RadioControls,
    sampler::Source,
    transport::{Broker, Bus, Discovery},
};
#[path = "support/band_templates.rs"]
mod band_templates;
#[path = "support/temperature_cases.rs"]
mod temperature_cases;

#[test]
fn sierra_templates_use_the_same_verified_reads_and_generated_writes_in_both_modes() {
    use std::collections::BTreeMap;
    for model in ["em9190", "em9291", "mc7354", "mc7355"] {
        let lte = catalog::platform(model) == "lte";
        let mut direct_replies = BTreeMap::new();
        let mut shared_replies = BTreeMap::new();
        for (command, reply) in [
            ("AT!BAND?", band_templates::current(lte, false)),
            ("AT!BAND=?", band_templates::available(lte)),
        ] {
            let (_, direct, shared) = sources(model, &reply);
            let a = direct.modems().unwrap().remove(0);
            let b = shared.modems().unwrap().remove(0);
            direct_replies.insert(command.into(), direct.query_bound(&a, command).unwrap());
            shared_replies.insert(
                command.into(),
                Source::query_bound(&shared, &b, command).unwrap(),
            );
        }
        let a = wlmb_server::band_locks::parse(model, &direct_replies).unwrap();
        let b = wlmb_server::band_locks::parse(model, &shared_replies).unwrap();
        assert_eq!(a, b);
        let op = if lte {
            json!({"rat":"LTE","mode":"band","entries":[{"band":3}]})
        } else {
            json!({"rat":"NR","mode":"band","entries":[{"band":78}]})
        };
        assert_eq!(
            wlmb_server::band_locks::commands(model, &op, &a).unwrap(),
            wlmb_server::band_locks::commands(model, &op, &b).unwrap()
        );
    }
}

#[test]
fn all_registered_temperature_protocols_read_the_same_in_shared_and_independent_modes() {
    let mut count = 0;
    for (_, models) in catalog::MODELS {
        for model in models.split_whitespace() {
            if wlmb_server::temperature::queries(model).is_empty() {
                continue;
            }
            count += 1;
            for command in wlmb_server::temperature::queries(model) {
                let (raw, expected) = temperature_cases::reply(model, command);
                let (_, direct, shared) = sources(model, raw);
                let a = direct.modems().unwrap().remove(0);
                let b = shared.modems().unwrap().remove(0);
                let x = direct.query_bound(&a, command).unwrap();
                let y = Source::query_bound(&shared, &b, command).unwrap();
                let x = wlmb_server::temperature::response(model, command, &x).unwrap();
                let y = wlmb_server::temperature::response(model, command, &y).unwrap();
                assert_eq!(x, y, "{model} {command}");
                assert_eq!(x["celsius"], expected);
            }
        }
    }
    assert_eq!(count, 89);
}
struct Fixture {
    model: String,
    reply: String,
    calls: Mutex<Vec<String>>,
}
impl Fixture {
    fn row(&self, shared: bool) -> Value {
        json!({"id":"fixture","port":"/dev/ttyUSB1","model":self.model,"instance":"fixture:1","shared":shared})
    }
}
struct Port(Arc<Fixture>);
impl Endpoint for Port {
    fn query(&mut self, command: &str, _timeout: Duration) -> Result<String> {
        self.0.calls.lock().unwrap().push(command.into());
        if command == "AT+CGMM" {
            return Ok(format!("+CGMM: {}\r\nOK\r\n", self.0.model));
        }
        Ok(self.0.reply.clone())
    }
    fn send_pdu(&mut self, _: &str, _: usize) -> Result<String> {
        panic!("Read-only parity test must never send SMS")
    }
    fn quarantined(&self) -> bool {
        false
    }
}
struct Board(Arc<Fixture>);
impl Hardware for Board {
    fn inventory(&self) -> Result<Vec<Value>> {
        Ok(vec![self.0.row(false)])
    }
    fn owners(&self, _: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    fn open(&self, _: &str) -> Result<Box<dyn Endpoint>> {
        Ok(Box::new(Port(self.0.clone())))
    }
}
struct Shared(Arc<Fixture>);
impl Discovery for Shared {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![self.0.row(true)])
    }
    fn owners(&self, _: &str) -> Result<Vec<String>> {
        Ok(vec!["ubus-at-daemon".into()])
    }
}
impl Bus for Shared {
    fn call(&self, object: &str, method: &str, params: Value, _timeout: Duration) -> Result<Value> {
        assert_eq!(object, "at-daemon");
        if method == "list" {
            return Ok(json!({"ports":[{"port":"/dev/ttyUSB1","is_open":1}]}));
        }
        assert_eq!(method, "sendat");
        self.0
            .calls
            .lock()
            .unwrap()
            .push(params["at_cmd"].as_str().unwrap().into());
        Ok(json!({"status":"success","response":self.0.reply}))
    }
}
fn sources(model: &str, reply: &str) -> (Arc<Fixture>, Arc<IndependentSource>, Broker) {
    let fixture = Arc::new(Fixture {
        model: model.into(),
        reply: reply.into(),
        calls: Mutex::new(vec![]),
    });
    let direct = IndependentSource::new(Arc::new(Board(fixture.clone())));
    let shared = Arc::new(Shared(fixture.clone()));
    (fixture, direct, Broker::new(shared.clone(), shared))
}
#[test]
fn all_91_models_share_the_same_catalog_operator_bands_and_typed_control_scope() {
    let mut count = 0;
    for (_, models) in catalog::MODELS {
        for model in models.split_whitespace() {
            let (fixture, direct, shared) =
                sources(model, "+COPS: 0,0,\"CHN-UNICOM\",12\r\nOK\r\n");
            let a = direct.modems().unwrap().remove(0);
            let b = shared.modems().unwrap().remove(0);
            assert_eq!(
                catalog::identify(a["model"].as_str().unwrap()),
                catalog::identify(b["model"].as_str().unwrap()),
                "{model}"
            );
            assert_eq!(
                controls::presets(a["model"].as_str().unwrap()),
                controls::presets(b["model"].as_str().unwrap())
            );
            assert_eq!(RadioControls::supported(&a), RadioControls::supported(&b));
            let x = direct.query_bound(&a, "AT+COPS?").unwrap();
            let y = Source::query_bound(&shared, &b, "AT+COPS?").unwrap();
            let profile = catalog::identify(model);
            let parser = profile["parser"].as_str().unwrap_or("generic");
            assert_eq!(
                radio::parse(parser, &x).unwrap(),
                radio::parse(parser, &y).unwrap()
            );
            assert_eq!(radio::parse(parser, &x).unwrap()["operator"], "中国联通");
            assert!(
                fixture
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|c| c == "AT+CGMM" || c == "AT+COPS?")
            );
            count += 1;
        }
    }
    assert_eq!(count, 91);
}
#[test]
fn implemented_profiles_dispatch_the_same_read_queries_in_both_modes() {
    let mut profiles = 0;
    for (_, models) in catalog::MODELS {
        for model in models.split_whitespace() {
            let profile = catalog::identify(model);
            if !profile["parser"].is_string() {
                continue;
            }
            let (fixture, direct, shared) = sources(model, "OK\r\n");
            let a = direct.modems().unwrap().remove(0);
            let b = shared.modems().unwrap().remove(0);
            for command in profile["queries"].as_array().unwrap() {
                let command = command.as_str().unwrap();
                assert!(
                    wlmb_server::security::read_only(command),
                    "{model}: {command}"
                );
                assert_eq!(
                    direct.query_bound(&a, command).unwrap(),
                    Source::query_bound(&shared, &b, command).unwrap(),
                    "{model}: {command}"
                );
            }
            assert!(fixture.calls.lock().unwrap().iter().all(|command| {
                command == "AT+CGMM"
                    || profile["queries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|q| q == command)
            }));
            profiles += 1;
        }
    }
    assert!(
        profiles > 60,
        "Do not replace exhaustive profile coverage with MT5700 only"
    );
}
#[test]
fn representative_radio_replies_keep_identical_measurements_across_transports() {
    let cases = [
        (
            "mt5700m-cn",
            "AT^MONSC",
            "^MONSC: NR,460,01,627264,1,abc,1E8,abc,-80,-5,16\r\nOK\r\n",
        ),
        (
            "rm520n-gl",
            "AT+QENG=\"servingcell\"",
            "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,01,5F1EA15,12,1650,3,5,5,DE10,-100,-12,-68,11\r\nOK\r\n",
        ),
        (
            "rm500u-cn",
            "AT+QENG=\"servingcell\"",
            "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,01,5F1EA15,12,1650,3,5,5,DE10,-100,-12,-68,11\r\nOK\r\n",
        ),
        (
            "simcom-sim8200ea-m2",
            "AT+CPSI?",
            "+CPSI: LTE,Online,460-11,0x1111,1234567,123,EUTRAN-BAND3,1850,5,4,-90,-850,-650,15\r\nOK\r\n",
        ),
        (
            "nl668",
            "AT+GTCCINFO?",
            "+GTCCINFO:\r\nLTE service cell:\r\n1,4,460,01,6334,140542123,39148,351,40,5,14,51,51,29\r\nOK\r\n",
        ),
    ];
    for (model, command, reply) in cases {
        let (_, direct, shared) = sources(model, reply);
        let a = direct.modems().unwrap().remove(0);
        let b = shared.modems().unwrap().remove(0);
        let x = direct.query_bound(&a, command).unwrap();
        let y = Source::query_bound(&shared, &b, command).unwrap();
        let profile = catalog::identify(model);
        let parser = profile["parser"].as_str().unwrap();
        let x = radio::parse(parser, &x).unwrap();
        let y = radio::parse(parser, &y).unwrap();
        assert!(!x["serving"].as_array().unwrap().is_empty(), "{model}");
        assert_eq!(x, y, "{model}");
    }
}
#[test]
fn shared_sms_read_only_is_a_transport_boundary_not_a_missing_model_parser() {
    use wlmb_server::{compat_sms::CompatSms, sms_runtime::SmsDevice};
    let (_, direct, shared) = sources("mt5700m-cn", "OK\r\n");
    let shared = CompatSms::new(Arc::new(shared));
    assert!(!shared.writable());
    assert!(direct.writable());
    assert_eq!(
        shared.send("fixture", "10086", "never sent").unwrap_err(),
        PanelError::new("SMS_SHARED_READ_ONLY", 422)
    );
}
