#![cfg(unix)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
#![allow(clippy::unwrap_used)]
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use wlmb_server::{
    PanelError, Result,
    compat_sms::CompatSms,
    sms_runtime::SmsDevice,
    transport::{Broker, Bus, Discovery},
};
fn schema() -> Value {
    json!({"sendat":{"at_port":3,"at_cmd":3,"raw_at_content":3,"end_flag":3,"owner":3,"timeout":5},"lease_acquire":{"at_port":3,"owner":3,"ttl":5},"lease_release":{"at_port":3,"owner":3,"token":3}})
}
struct Owner {
    calls: Mutex<Vec<Value>>,
    legacy: bool,
    busy: bool,
    fail_payload: bool,
    changed: Mutex<bool>,
    change_on_prompt: bool,
    lease_owner: Mutex<Option<String>>,
    queue: Option<Arc<wlmb_server::qmodem_queue::Contract>>,
    fail_abort: bool,
    model: String,
}
impl Bus for Owner {
    fn describe(&self, _: &str, _: Duration) -> Result<Value> {
        Ok(if self.legacy {
            json!({"sendat":{"at_port":3,"at_cmd":3,"raw_at_content":3,"end_flag":3,"timeout":5}})
        } else {
            schema()
        })
    }
    fn call(&self, object: &str, method: &str, args: Value, _: Duration) -> Result<Value> {
        assert_eq!(object, "at-daemon");
        self.calls
            .lock()
            .unwrap()
            .push(json!({"method":method,"args":args}));
        if method == "list" {
            return Ok(json!({"ports":[{"port":"/dev/ttyUSB1","is_open":1}]}));
        }
        if method == "lease_acquire" {
            if self.busy {
                return Ok(json!({"status":"busy"}));
            }
            *self.lease_owner.lock().unwrap() = Some(args["owner"].as_str().unwrap().into());
            assert_eq!(args["ttl"], 120);
            return Ok(json!({"status":"success","token":"1-a"}));
        }
        assert_eq!(
            args["owner"].as_str(),
            self.lease_owner.lock().unwrap().as_deref()
        );
        if method == "lease_release" {
            *self.lease_owner.lock().unwrap() = None;
            return Ok(json!({"status":"success"}));
        }
        assert_eq!(method, "sendat");
        if let Some(queue) = &self.queue {
            assert!(
                queue.acquire("/dev/ttyUSB1").is_err(),
                "All AT phases must hold both external locks"
            );
        }
        let command = args["at_cmd"].as_str().unwrap_or("");
        let raw: String = if let Some(payload) = args["raw_at_content"].as_str() {
            assert!(payload.len() % 2 == 0 && payload.bytes().all(|b| b.is_ascii_hexdigit()));
            let decoded: Vec<u8> = (0..payload.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&payload[i..i + 2], 16).unwrap())
                .collect();
            if decoded == [0x1b] {
                if self.fail_abort {
                    return Err(PanelError::new("SYSTEM_UNAVAILABLE", 503));
                }
                "OK\n".into()
            } else {
                assert_eq!(decoded.last(), Some(&0x1a));
                assert!(
                    decoded[..decoded.len() - 1]
                        .iter()
                        .all(u8::is_ascii_hexdigit)
                );
                if self.fail_payload {
                    return Err(PanelError::new("SYSTEM_UNAVAILABLE", 503));
                }
                "+CMGS: 7\nOK\n".into()
            }
        } else if command.starts_with("AT+CMGS=") {
            assert_eq!(args["end_flag"], ">");
            if self.change_on_prompt {
                *self.changed.lock().unwrap() = true;
            }
            ">".into()
        } else {
            match command {
                "AT+CMGF?" => "+CMGF: 0\nOK\n".into(),
                "AT+CPMS?" => "+CPMS: \"ME\",1,100,\"ME\",1,100,\"ME\",1,100\nOK\n".into(),
                "AT+CMGL=4" => {
                    "+CMGL: 1,0,,18\n000005810080F6000862010121436523024F60\nOK\n".into()
                }
                "AT+CMGD=1,0" => "OK\n".into(),
                "AT+COPS?" => "+COPS: 0,2,\"46001\",7\nOK\n".into(),
                _ => panic!("Unexpected {command}"),
            }
        };
        Ok(json!({"status":"success","response":raw}))
    }
}
impl Discovery for Owner {
    fn shared_queue(&self) -> Option<Arc<wlmb_server::qmodem_queue::Contract>> {
        self.queue.clone()
    }
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![
            json!({"id":"m1","model":self.model,"port":"/dev/ttyUSB1","shared":true,"instance":if *self.changed.lock().unwrap(){2}else{1}}),
        ])
    }
    fn owners(&self, _: &str) -> Result<Vec<String>> {
        Ok(vec!["ubus-at-daemon".into()])
    }
}
fn setup(
    legacy: bool,
    busy: bool,
    fail_payload: bool,
    change_on_prompt: bool,
) -> (CompatSms, Arc<Owner>) {
    let owner = Arc::new(Owner {
        calls: Mutex::new(vec![]),
        legacy,
        busy,
        fail_payload,
        changed: Mutex::new(false),
        change_on_prompt,
        lease_owner: Mutex::new(None),
        queue: None,
        fail_abort: false,
        model: "mt5700m-cn".into(),
    });
    (
        CompatSms::new(Arc::new(Broker::new(owner.clone(), owner.clone()))),
        owner,
    )
}
#[test]
fn old_shared_service_never_dispatches_unsafe_sends_or_deletes() {
    let (sms, owner) = setup(true, false, false, false);
    assert!(!sms.writable());
    assert_eq!(
        sms.send("m1", "10086", "hello").unwrap_err().code,
        "SMS_SHARED_READ_ONLY"
    );
    assert!(sms.delete("m1", &"a".repeat(64)).is_err());
    assert!(owner.calls.lock().unwrap().is_empty());
}
#[test]
fn shared_send_is_one_leased_prompt_payload_transaction_without_mode_mutation() {
    let (sms, owner) = setup(false, false, false, false);
    assert!(sms.writable());
    assert_eq!(sms.send("m1", "10086", "hello").unwrap()["reference"], 7);
    let calls = owner.calls.lock().unwrap();
    assert_eq!(calls.first().unwrap()["method"], "lease_acquire");
    assert_eq!(calls.last().unwrap()["method"], "lease_release");
    assert_eq!(
        calls
            .iter()
            .filter(|c| c["args"]["raw_at_content"].is_string())
            .count(),
        1
    );
    assert!(!calls.iter().any(|c| {
        c["args"]["at_cmd"]
            .as_str()
            .is_some_and(|v| v.starts_with("AT+CMGF="))
    }));
}
#[test]
fn lease_busy_or_hotplug_blocks_payload_and_never_retries() {
    let (sms, owner) = setup(false, true, false, false);
    assert_eq!(
        sms.send("m1", "10086", "test").unwrap_err().code,
        "SMS_SHARED_BUSY"
    );
    assert_eq!(owner.calls.lock().unwrap().len(), 1);
    let (sms, owner) = setup(false, false, false, true);
    assert!(sms.send("m1", "10086", "test").is_err());
    assert!(
        !owner
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c["args"]["raw_at_content"].is_string())
    );
}
#[test]
fn payload_failure_only_cancels_and_does_not_resend() {
    let (sms, owner) = setup(false, false, true, false);
    assert!(sms.send("m1", "10086", "test").is_err());
    let calls = owner.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| c["args"]["raw_at_content"]
                .as_str()
                .is_some_and(|v| v.ends_with("1A")))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| c["args"]["raw_at_content"] == "1B")
            .count(),
        1
    );
}
#[test]
fn shared_delete_revalidates_fingerprint_and_storage_under_one_lease() {
    let (sms, owner) = setup(false, false, false, false);
    let record = wlmb_server::sms_pdu::parse_cmgl(
        "+CMGL: 1,0,,18\n000005810080F6000862010121436523024F60\nOK\n",
        "m1",
        "ME",
    )
    .unwrap();
    assert_eq!(
        sms.delete("m1", &record[0].message.fingerprint().unwrap())
            .unwrap()["parts"],
        1
    );
    let calls = owner.calls.lock().unwrap();
    assert_eq!(calls.first().unwrap()["method"], "lease_acquire");
    assert_eq!(calls.last().unwrap()["method"], "lease_release");
    assert_eq!(
        calls
            .iter()
            .filter(|c| c["args"]["at_cmd"] == "AT+CMGD=1,0")
            .count(),
        1
    );
}
#[test]
fn partial_schema_or_wrong_policy_types_do_not_imply_transactions() {
    assert!(wlmb_server::shared_sms::supported(&schema()));
    let mut incomplete = schema();
    incomplete["sendat"]
        .as_object_mut()
        .unwrap()
        .remove("owner");
    assert!(!wlmb_server::shared_sms::supported(&incomplete));
    let mut wrong = schema();
    wrong["lease_acquire"]["ttl"] = json!(3);
    assert!(!wlmb_server::shared_sms::supported(&wrong));
}
#[test]
fn raw_transport_encodes_ascii_pdu_plus_control_byte_not_pdu_octets() {
    assert_eq!(
        wlmb_server::shared_sms::wire_raw("00AB\u{1a}").unwrap(),
        "303041421A"
    );
    assert_eq!(wlmb_server::shared_sms::wire_raw("\u{1b}").unwrap(), "1B");
    for payload in ["".to_owned(), "中".to_owned(), "0".repeat(2049)] {
        assert!(wlmb_server::shared_sms::wire_raw(&payload).is_err());
    }
}
struct QueueFixture {
    root: std::path::PathBuf,
    queue: Arc<wlmb_server::qmodem_queue::Contract>,
}
impl QueueFixture {
    fn new() -> Self {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let root =
            std::env::temp_dir().join(format!("wlmb-sms-queue-{:x}", u128::from_le_bytes(random)));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let source = root.join("reviewed");
        std::fs::write(&source, b"reviewed").unwrap();
        let queue = Arc::new(
            wlmb_server::qmodem_queue::Contract::verify(
                &root,
                &[(&source, &format!("{:x}", Sha256::digest(b"reviewed")))],
                std::fs::metadata(&root).unwrap().uid(),
            )
            .unwrap(),
        );
        Self { root, queue }
    }
    fn owner(&self, fail: bool) -> Arc<Owner> {
        Arc::new(Owner {
            calls: Mutex::new(vec![]),
            legacy: true,
            busy: false,
            fail_payload: fail,
            changed: Mutex::new(false),
            change_on_prompt: false,
            lease_owner: Mutex::new(None),
            queue: Some(self.queue.clone()),
            fail_abort: fail,
            model: "mt5700m-cn".into(),
        })
    }
}
impl Drop for QueueFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn reviewed_legacy_queue_serializes_queries_and_entire_sms_without_owner_upgrade() {
    let f = QueueFixture::new();
    let owner = f.owner(false);
    let broker = Arc::new(Broker::new(owner.clone(), owner.clone()));
    let sms = CompatSms::new(broker.clone());
    assert!(sms.writable());
    let held = f.queue.acquire("/dev/ttyUSB1").unwrap();
    assert_eq!(
        broker.query("m1", "AT+COPS?", None).unwrap_err().code,
        "AT_BUSY"
    );
    assert!(owner.calls.lock().unwrap().is_empty());
    drop(held);
    assert!(broker.query("m1", "AT+COPS?", None).is_ok());
    assert_eq!(sms.send("m1", "10086", "hello").unwrap()["reference"], 7);
    let calls = owner.calls.lock().unwrap();
    assert!(
        !calls
            .iter()
            .any(|c| c["method"] == "lease_acquire" || c["method"] == "lease_release")
    );
    assert!(!calls.iter().any(|c| c["args"]["owner"].is_string()));
    drop(calls);
    assert!(f.queue.acquire("/dev/ttyUSB1").is_ok());
}
#[test]
fn uncertain_legacy_input_retains_external_locks_and_closes_own_capabilities() {
    let f = QueueFixture::new();
    let owner = f.owner(true);
    let broker = Arc::new(Broker::new(owner.clone(), owner.clone()));
    let sms = CompatSms::new(broker.clone());
    assert!(sms.send("m1", "10086", "hello").is_err());
    assert_eq!(broker.modems().unwrap()[0]["channelReady"], false);
    assert!(f.queue.acquire("/dev/ttyUSB1").is_err());
    assert_eq!(
        broker.query("m1", "AT+COPS?", None).unwrap_err().code,
        "SMS_INPUT_UNKNOWN"
    );
    let calls = owner.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| c["args"]["raw_at_content"]
                .as_str()
                .is_some_and(|s| s.ends_with("1A")))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| c["args"]["raw_at_content"] == "1B")
            .count(),
        1
    );
    drop(calls);
    drop(sms);
    drop(broker);
    assert!(f.queue.acquire("/dev/ttyUSB1").is_ok());
}
#[test]
fn standard_sms_transactions_share_the_same_guard_for_all_catalog_models() {
    let f = QueueFixture::new();
    for (_, models) in wlmb_server::catalog::MODELS {
        for model in models.split_whitespace() {
            let mut owner = f.owner(false);
            Arc::get_mut(&mut owner).unwrap().model = model.into();
            let sms = CompatSms::new(Arc::new(Broker::new(owner.clone(), owner.clone())));
            assert_eq!(
                sms.send("m1", "10086", "hello").unwrap()["reference"],
                7,
                "{model}"
            );
            let record = wlmb_server::sms_pdu::parse_cmgl(
                "+CMGL: 1,0,,18\n000005810080F6000862010121436523024F60\nOK\n",
                "m1",
                "ME",
            )
            .unwrap();
            assert_eq!(
                sms.delete("m1", &record[0].message.fingerprint().unwrap())
                    .unwrap()["parts"],
                1,
                "{model}"
            );
            assert!(
                !owner
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|c| c["method"] == "lease_acquire"
                        || c["method"] == "open"
                        || c["method"] == "close")
            );
        }
    }
}
