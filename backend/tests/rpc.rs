#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
#[cfg(unix)]
mod linux {
    use serde_json::{Value, json};
    use std::time::Duration;
    use wlmb_server::{Result, rpc, transport::Bus};
    struct Mock(bool);
    impl Bus for Mock {
        fn call(
            &self,
            object: &str,
            method: &str,
            args: Value,
            _timeout: Duration,
        ) -> Result<Value> {
            assert_eq!((object, method), ("session", "access"));
            assert_eq!(args["scope"], "ubus");
            assert_eq!(args["object"], "network-panel");
            assert_eq!(args["function"], "service_control");
            Ok(json!({"access":self.0}))
        }
    }
    #[test]
    fn rpc_acl_is_required_and_method_specific() {
        assert_eq!(
            rpc::authorize(&Mock(true), "service_status", &json!({}))
                .err()
                .map(|error| error.code),
            Some("SESSION_EXPIRED")
        );
        assert!(
            rpc::authorize(
                &Mock(true),
                "service_control",
                &json!({"ubus_rpc_session":"a".repeat(32)})
            )
            .is_ok()
        );
        assert!(
            rpc::authorize(
                &Mock(false),
                "service_control",
                &json!({"ubus_rpc_session":"a".repeat(32)})
            )
            .is_err()
        );
        assert!(rpc::authorize(&Mock(true), "service_control", &json!({})).is_err());
        assert!(
            rpc::authorize(
                &Mock(true),
                "shell",
                &json!({"ubus_rpc_session":"a".repeat(32)})
            )
            .is_err()
        );
    }
    #[test]
    fn lifecycle_accepts_only_fixed_own_service_actions() {
        for action in ["start", "stop", "restart"] {
            assert_eq!(
                rpc::operation(&json!({"action":action})).unwrap(),
                (action, None)
            );
        }
        assert_eq!(
            rpc::operation(&json!({"action":"boot","enabled":false})).unwrap(),
            ("boot", Some(false))
        );
        for args in [
            json!({"action":"reboot"}),
            json!({"action":"start; reboot"}),
            json!({"action":"boot","enabled":"1"}),
        ] {
            assert!(rpc::operation(&args).is_err());
        }
    }
}
