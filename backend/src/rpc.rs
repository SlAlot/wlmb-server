//! LuCI的rpcd方法描述与请求分派，复用服务层权限和操作确认。
use crate::{
    PanelError, Result, VERSION, boundary,
    service::NativeService,
    transport::{self, Bus, NativeBus},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub const DIRECTORY: &str = "/var/run/wlmb-server";
pub const SOCKET: &str = "/var/run/wlmb-server/control.sock";
fn unavailable() -> PanelError {
    PanelError::new("SERVICE_UNAVAILABLE", 503)
}
pub fn methods() -> Value {
    json!({
        "status":{"modem":"","ubus_rpc_session":""},
        "settings":{"ubus_rpc_session":""},"diagnostics":{"ubus_rpc_session":""},
        "service_status":{"ubus_rpc_session":""},
        "service_control":{"action":"","enabled":false,"ubus_rpc_session":""},
        "refresh":{"modem":"","ubus_rpc_session":""},
        "configure":{"interval":32,"allowUnsafeAt":false,"ubus_rpc_session":""},
        "at_prepare":{"modem":"","command":"","ubus_rpc_session":""},
        "at_execute":{"modem":"","command":"","confirmation":"","ubus_rpc_session":""},
        "lock_status":{"modem":"","force":false,"ubus_rpc_session":""},
        "lock_prepare":{"modem":"","operation":"","ubus_rpc_session":""},
        "lock_execute":{"modem":"","operation":"","revision":"","confirmation":"","ubus_rpc_session":""},
        "host_prepare":{"operation":"","ubus_rpc_session":""},
        "host_execute":{"operation":"","revision":"","confirmation":"","ubus_rpc_session":""},
        "control_result":{"job":"","ubus_rpc_session":""},
        "sms_list":{"modem":"","ubus_rpc_session":""},
        "sms_settings_prepare":{"modem":"","enabled":false,"ubus_rpc_session":""},
        "sms_settings_execute":{"modem":"","enabled":false,"confirmation":"","ubus_rpc_session":""},
        "sms_refresh":{"modem":"","ubus_rpc_session":""},
        "sms_read_state":{"modem":"","id":"","unread":false,"ubus_rpc_session":""},
        "sms_prepare":{"modem":"","operation":"","ubus_rpc_session":""},
        "sms_execute":{"modem":"","operation":"","confirmation":"","ubus_rpc_session":""},
        "sms_forward_settings":{"ubus_rpc_session":""},
        "sms_forward_configure":{"enabled":false,"receiveEnabled":false,"modem":"","pollSeconds":10,"token":"","clearToken":false,"ubus_rpc_session":""}
    })
}
pub fn authorize(bus: &dyn Bus, method: &str, args: &Value) -> Result<()> {
    if methods().get(method).is_none() {
        return Err(PanelError::new("METHOD_DENIED", 403));
    }
    let owner = args["ubus_rpc_session"].as_str().unwrap_or("");
    if owner.len() != 32 || !owner.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(PanelError::new("SESSION_EXPIRED", 401));
    }
    let allowed = bus.call(
        "session",
        "access",
        json!({"ubus_rpc_session":owner,"scope":"ubus","object":"network-panel","function":method}),
        Duration::from_secs(3),
    )?;
    if allowed["access"] != true {
        return Err(PanelError::new("METHOD_DENIED", 403));
    }
    Ok(())
}
fn envelope(result: Result<Value>) -> Value {
    match result {
        Ok(data) => json!({"ok":true,"data":data}),
        Err(e) => {
            json!({"ok":false,"error":e.details()})
        }
    }
}
fn validate_socket() -> Result<()> {
    let dir = fs::symlink_metadata(DIRECTORY).map_err(|_| unavailable())?;
    let socket = fs::symlink_metadata(SOCKET).map_err(|_| unavailable())?;
    if !dir.is_dir()
        || dir.uid() != 0
        || dir.permissions().mode() & 0o777 != 0o700
        || !socket.file_type().is_socket()
        || socket.uid() != 0
        || socket.permissions().mode() & 0o777 != 0o600
    {
        return Err(unavailable());
    }
    Ok(())
}
pub fn request(method: &str, args: Value) -> Result<Value> {
    validate_socket()?;
    let mut stream = UnixStream::connect(SOCKET).map_err(|_| unavailable())?;
    let deadline = Instant::now() + Duration::from_secs(4);
    transport::write_frame(&mut stream, &json!({"method":method,"args":args}), deadline)?;
    transport::read_frame(&mut stream, deadline)
}
fn validate_health(reply: &Value) -> Result<()> {
    if reply["ok"] == true
        && reply["data"]["backend"] == "rust"
        && reply["data"]["version"] == VERSION
    {
        Ok(())
    } else {
        Err(unavailable())
    }
}
/// 通过仅限根用户的本地套接字确认同版服务响应，不经过需要登录的网页接口。
pub fn check_ready() -> Result<()> {
    validate_health(&request("ping", json!({}))?)
}
fn runtime() -> Value {
    let ready = check_ready().is_ok();
    let boot = transport::process(
        "/sbin/uci",
        &["-q", "get", "network_panel.main.boot_enabled"],
        None,
        Duration::from_secs(2),
    )
    .is_ok_and(|v| v.trim() == "1");
    json!({"name":"wlmb-server","title":"网络面板","version":VERSION,"backend":"rust","running":ready,"controlReady":ready,"enabled":ready,"bootEnabled":boot,"apiPort":1117,"development":true})
}
pub fn operation(args: &Value) -> Result<(&str, Option<bool>)> {
    let action = args["action"]
        .as_str()
        .ok_or_else(|| PanelError::new("INVALID_CONTROL", 400))?;
    match action {
        "start" | "stop" | "restart" => Ok((action, None)),
        "boot" => Ok((
            action,
            Some(
                args["enabled"]
                    .as_bool()
                    .ok_or_else(|| PanelError::new("INVALID_CONTROL", 400))?,
            ),
        )),
        _ => Err(PanelError::new("INVALID_CONTROL", 400)),
    }
}
fn lifecycle(args: &Value) -> Result<Value> {
    let (action, enabled) = operation(args)?;
    if let Some(boot) = enabled {
        transport::process(
            "/sbin/uci",
            &[
                "set",
                if boot {
                    "network_panel.main.boot_enabled=1"
                } else {
                    "network_panel.main.boot_enabled=0"
                },
            ],
            None,
            Duration::from_secs(3),
        )?;
        transport::process(
            "/sbin/uci",
            &["commit", "network_panel"],
            None,
            Duration::from_secs(3),
        )?;
        transport::process(
            "/etc/init.d/wlmb-server",
            &[if boot { "enable" } else { "disable" }],
            None,
            Duration::from_secs(3),
        )?;
    } else {
        transport::process(
            "/etc/init.d/wlmb-server",
            &[action],
            None,
            Duration::from_secs(8),
        )?;
    }
    Ok(json!({"accepted":true,"service":runtime()}))
}
pub fn cli() -> Result<()> {
    let argv = std::env::args().skip(1).collect::<Vec<_>>();
    if argv == ["list"] {
        println!("{}", methods());
        return Ok(());
    }
    if argv.len() != 2 || argv[0] != "call" {
        return Err(PanelError::new("INVALID_CONTROL", 400));
    }
    let mut raw = vec![];
    std::io::stdin()
        .take(16385)
        .read_to_end(&mut raw)
        .map_err(|_| unavailable())?;
    let args = boundary::decode_json(&raw)?;
    let bus = NativeBus::default();
    let result = authorize(&bus, &argv[1], &args).and_then(|()| match argv[1].as_str() {
        "service_status" => Ok(envelope(Ok(runtime()))),
        "service_control" => Ok(envelope(lifecycle(&args))),
        method => request(method, args),
    });
    println!("{}", result.unwrap_or_else(|e| envelope(Err(e))));
    Ok(())
}
pub fn serve(
    service: Arc<NativeService>,
    stop: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>> {
    if !Path::new(DIRECTORY).exists() {
        fs::create_dir(DIRECTORY).map_err(|_| unavailable())?;
        fs::set_permissions(DIRECTORY, fs::Permissions::from_mode(0o700))
            .map_err(|_| unavailable())?;
    }
    let dir = fs::symlink_metadata(DIRECTORY).map_err(|_| unavailable())?;
    if !dir.is_dir() || dir.uid() != 0 || dir.permissions().mode() & 0o777 != 0o700 {
        return Err(unavailable());
    }
    if Path::new(SOCKET).exists() {
        let meta = fs::symlink_metadata(SOCKET).map_err(|_| unavailable())?;
        if !meta.file_type().is_socket() || meta.uid() != 0 || UnixStream::connect(SOCKET).is_ok() {
            return Err(unavailable());
        }
        fs::remove_file(SOCKET).map_err(|_| unavailable())?;
    }
    let listener = UnixListener::bind(SOCKET).map_err(|_| unavailable())?;
    fs::set_permissions(SOCKET, fs::Permissions::from_mode(0o600)).map_err(|_| unavailable())?;
    listener.set_nonblocking(true).map_err(|_| unavailable())?;
    Ok(std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let deadline = Instant::now() + Duration::from_secs(3);
                    let result = transport::read_frame(&mut stream, deadline).and_then(|v| {
                        if !v["args"].is_object() {
                            return Err(PanelError::new("INVALID_JSON", 400));
                        }
                        service.local(v["method"].as_str().unwrap_or(""), v["args"].clone())
                    });
                    let raw = serde_json::to_vec(&envelope(result));
                    if let Ok(raw) = raw {
                        if raw.len() > 262144 {
                            continue;
                        }
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let _ = stream
                            .write_all(format!("{:08x}", raw.len()).as_bytes())
                            .and_then(|_| stream.write_all(&raw));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(_) => break,
            }
        }
        let _ = fs::remove_file(SOCKET);
    }))
}

#[cfg(test)]
mod health_tests {
    use super::*;

    #[test]
    fn readiness_requires_successful_same_version_rust_response() {
        assert!(
            validate_health(&json!({"ok":true,"data":{"backend":"rust","version":VERSION}}))
                .is_ok()
        );
        for reply in [
            json!(null),
            json!({"data":{"running":true}}),
            json!({"ok":false,"error":{"code":"SESSION_EXPIRED"}}),
            json!({"ok":true,"data":{"backend":"rust","version":"0.0.0"}}),
            json!({"ok":true,"data":{"backend":"rust"}}),
            json!({"ok":true,"data":{"backend":"other","version":VERSION}}),
            json!({"ok":false,"data":{"backend":"rust","version":VERSION}}),
        ] {
            assert!(validate_health(&reply).is_err());
        }
    }
}
