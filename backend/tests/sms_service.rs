#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
#[cfg(unix)]
mod linux {
    use serde_json::{Value, json};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };
    use wlmb_server::{
        PanelError, Result,
        sampler::{Sampler, Source},
        service::NativeService,
        sms_pdu::Record,
        sms_runtime::{SmsDevice, SmsHub},
        transport::Bus,
    };
    struct Empty;
    impl Source for Empty {
        fn modems(&self) -> Result<Vec<Value>> {
            Ok(vec![])
        }
        fn query(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String> {
            Err(PanelError::new("AT_UNSUPPORTED", 422))
        }
    }
    impl Bus for Empty {
        fn call(&self, _: &str, _: &str, _: Value, _: Duration) -> Result<Value> {
            Err(PanelError::new("UNEXPECTED_BUS_CALL", 500))
        }
    }
    struct Device {
        revision: AtomicUsize,
        sends: AtomicUsize,
        reads: AtomicUsize,
    }
    impl SmsDevice for Device {
        fn revision(&self, _: &str) -> Result<String> {
            Ok(self.revision.load(Ordering::SeqCst).to_string())
        }
        fn read(&self, _: &str) -> Result<Vec<Record>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        }
        fn send(&self, _: &str, _: &str, _: &str) -> Result<Value> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"sent":true}))
        }
    }
    fn setup() -> (Arc<Device>, Arc<NativeService>) {
        let d = Arc::new(Device {
            revision: AtomicUsize::new(1),
            sends: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
        });
        let service = NativeService::with_sms(
            Arc::new(Empty),
            Sampler::new(Arc::new(Empty)),
            Some(SmsHub::new(d.clone())),
            None,
        );
        (d, service)
    }
    fn args(owner: char) -> Value {
        json!({"ubus_rpc_session":owner.to_string().repeat(32),"modem":"m1","operation":{"type":"send","number":"10086","content":"synthetic"}})
    }
    fn receipt(service: &NativeService, id: &str, owner: char) -> Value {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let v = service
                .local(
                    "control_result",
                    json!({"ubus_rpc_session":owner.to_string().repeat(32),"job":id}),
                )
                .unwrap();
            if v["state"] != "pending" {
                return v;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn sms_confirmation_is_owner_operation_revision_bound_and_consumed() {
        let (d, s) = setup();
        let mut v = args('a');
        let c = s.local("sms_prepare", v.clone()).unwrap();
        v["confirmation"] = c["confirmation"].clone();
        let mut other = v.clone();
        other["ubus_rpc_session"] = json!("b".repeat(32));
        assert_eq!(
            s.local("sms_execute", other).err().unwrap().code,
            "AT_CONFIRMATION_REQUIRED"
        );
        assert_eq!(
            s.local("sms_execute", v.clone()).err().unwrap().code,
            "AT_CONFIRMATION_REQUIRED"
        );
        let c = s.local("sms_prepare", v.clone()).unwrap();
        v["confirmation"] = c["confirmation"].clone();
        d.revision.store(2, Ordering::SeqCst);
        assert!(s.local("sms_execute", v.clone()).is_err());
        assert_eq!(d.sends.load(Ordering::SeqCst), 0);
        let c = s.local("sms_prepare", v.clone()).unwrap();
        v["confirmation"] = c["confirmation"].clone();
        let job = s.local("sms_execute", v.clone()).unwrap();
        let id = job["job"].as_str().unwrap();
        assert_eq!(receipt(&s, id, 'a')["state"], "done");
        assert_eq!(d.sends.load(Ordering::SeqCst), 1);
        assert!(s.local("sms_execute", v).is_err());
        assert!(
            s.local(
                "control_result",
                json!({"ubus_rpc_session":"b".repeat(32),"job":id})
            )
            .is_err()
        );
    }
    #[test]
    fn cached_read_never_queries_and_rpc_json_operation_is_supported() {
        let (d, s) = setup();
        assert_eq!(
            s.local("sms_list", json!({"modem":"m1"})).unwrap()["loaded"],
            false
        );
        assert_eq!(d.reads.load(Ordering::SeqCst), 0);
        let mut v = args('a');
        v["operation"] = json!(v["operation"].to_string());
        let c = s.local("sms_prepare", v.clone()).unwrap();
        v["confirmation"] = c["confirmation"].clone();
        let job = s.local("sms_execute", v).unwrap();
        assert_eq!(
            receipt(&s, job["job"].as_str().unwrap(), 'a')["state"],
            "done"
        );
        assert!(s.local("sms_refresh", json!({"modem":"m1"})).is_err());
        let job = s
            .local(
                "sms_refresh",
                json!({"modem":"m1","ubus_rpc_session":"a".repeat(32)}),
            )
            .unwrap();
        let r = receipt(&s, job["job"].as_str().unwrap(), 'a');
        assert_eq!(r["result"]["count"], 0);
        assert!(r["result"].get("messages").is_none());
        assert_eq!(d.reads.load(Ordering::SeqCst), 1);
    }
}
