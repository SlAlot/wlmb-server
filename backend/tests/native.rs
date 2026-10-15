#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::json;
use wlmb_server::blob;

#[test]
fn native_blob_roundtrip_and_known_wire() {
    let value = json!({"name":"网络面板","up":true,"negative":-81,"long":37022989366_i64,"ratio":1.5,"null":null,"array":[1,false,{"PCI":488}],"empty":{}});
    let raw = blob::encode_object(&value).unwrap();
    assert_eq!(blob::decode_object(&raw).unwrap(), value);
    assert_eq!(
        blob::encode_object(&json!({"x":"a"})).unwrap(),
        vec![0x83, 0, 0, 10, 0, 1, b'x', 0, b'a', 0, 0, 0]
    );
    assert_eq!(
        blob::attr(3, false, &[0, 0, 0, 42]).unwrap(),
        vec![3, 0, 0, 8, 0, 0, 0, 42]
    );
}

mod http_transport {
    use serde_json::Value;
    use std::{
        io::{Read, Write},
        net::{Ipv4Addr, TcpListener, TcpStream},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };
    use wlmb_server::{
        Result,
        boundary::Lan,
        http::{self, Handler},
    };
    struct Echo(Arc<AtomicUsize>);
    impl Handler for Echo {
        fn handle(
            &self,
            _: &str,
            body: Value,
            _: &[(String, String)],
            _: Ipv4Addr,
        ) -> Result<Value> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(body)
        }
    }
    fn exchange(raw: &[u8]) -> (String, usize) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            http::connection(
                stream,
                Lan::new("192.168.1.1".parse().unwrap(), 24).unwrap(),
                "192.168.1.2".parse().unwrap(),
                &Echo(count),
            )
            .unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client.write_all(raw).unwrap();
        let mut out = String::new();
        if let Err(error) = client.read_to_string(&mut out) {
            assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
            assert!(out.starts_with("HTTP/1.1 "));
        }
        server.join().unwrap();
        (out, calls.load(Ordering::SeqCst))
    }
    #[test]
    fn actual_http_keepalive_pipeline_and_utf8_body() {
        let body = r#"{"name":"网络面板"}"#;
        let raw = format!(
            "POST /api/status HTTP/1.1\r\nHost: 192.168.1.1:1117\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}GET /health HTTP/1.1\r\nHost: 192.168.1.1:1117\r\nConnection: close\r\n\r\n",
            body.len(),
            body
        );
        let (out, count) = exchange(raw.as_bytes());
        assert_eq!(count, 2);
        assert_eq!(out.matches("HTTP/1.1 200").count(), 2);
        assert!(out.contains("网络面板"));
        assert!(out.contains("Cache-Control: no-store"));
    }
    #[test]
    fn actual_http_rebinding_and_smuggling_never_dispatch() {
        for headers in [
            "Host: evil.example:1117",
            "Host: 192.168.1.1:1117\r\nHost: evil.example:1117",
            "Host: 192.168.1.1:1117\r\nOrigin: http://evil.example",
            "Host: 192.168.1.1:1117\r\nContent-Length: 2",
            "Host: 192.168.1.1:1117\r\nTransfer-Encoding: chunked",
            "Host: 192.168.1.1:1117\r\nExpect: 100-continue",
        ] {
            let raw = format!(
                "POST /api/status HTTP/1.1\r\n{headers}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
            );
            let (out, count) = exchange(raw.as_bytes());
            assert_eq!(count, 0);
            assert!(!out.contains("HTTP/1.1 200"));
            assert!(out.contains("Connection: close"));
        }
    }
    #[test]
    fn actual_http_size_method_and_json_bounds() {
        for raw in [format!("GET /health HTTP/1.1\r\nHost: 192.168.1.1:1117\r\nX-Large: {}\r\n\r\n","x".repeat(8192)),"POST /api/status HTTP/1.1\r\nHost: 192.168.1.1:1117\r\nContent-Type: application/json\r\nContent-Length: 16385\r\n\r\n".into(),"POST /api/status HTTP/1.1\r\nHost: 192.168.1.1:1117\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n[]".into(),"PUT /api/status HTTP/1.1\r\nHost: 192.168.1.1:1117\r\n\r\n".into(),"GET /health HTTP/1.1\nHost: 192.168.1.1:1117\n\n".into()] {
            let (out,count)=exchange(raw.as_bytes());assert_eq!(count,0);assert!(!out.contains("HTTP/1.1 200"));
        }
    }
}
#[test]
fn native_blob_rejects_corruption_duplicates_and_depth() {
    let raw = blob::encode_object(&json!({"a":"value"})).unwrap();
    for size in 0..raw.len() {
        assert!(blob::decode_object(&raw[..size]).is_err() || size == 0);
    }
    let mut duplicate = raw.clone();
    duplicate.extend(&raw);
    assert!(blob::decode_object(&duplicate).is_err());
    for corrupt in [
        vec![0x82, 0, 0, 3],
        vec![0x83, 0, 0, 20, 0, 255, 0, 0],
        vec![0x83, 0, 0, 8, 0, 1, b'a', 0],
    ] {
        assert!(blob::decode_object(&corrupt).is_err());
    }
    let mut deep = json!(1);
    for _ in 0..26 {
        deep = json!({"x":deep});
    }
    assert!(blob::encode_object(&deep).is_err());
    assert!(blob::encode_object(&json!({"x":"embedded\u{0}null"})).is_err());
    assert!(blob::encode_object(&json!({"x":"a".repeat(blob::LIMIT)})).is_err());
}

#[cfg(unix)]
#[test]
#[ignore = "Only run inside the marked isolated OpenWrt guest, never on a supply router"]
fn isolated_native_http_login_status_and_logout() {
    use std::{
        io::{Read, Write},
        net::TcpStream,
        process::{Command, Stdio},
        sync::Arc,
        time::{Duration, Instant},
    };
    use wlmb_server::{service, transport::NativeBus};
    assert!(std::path::Path::new("/etc/network-panel-isolated-lab").is_file());
    assert!(
        !std::path::Path::new("/etc/config/qmodem").exists(),
        "No actual modem inventory in the isolation guest"
    );
    let binary = std::env::var("NP_NATIVE_TEST_BINARY").unwrap();
    assert!(binary.starts_with("/tmp/wlmb-rust.") && binary.ends_with("/core"));
    use std::os::unix::fs::DirBuilderExt;
    if !std::path::Path::new("/etc/wlmb-server").exists() {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create("/etc/wlmb-server")
            .unwrap();
    }
    wlmb_server::private_store::PrivateStore::open(std::path::Path::new("/etc/wlmb-server"))
        .unwrap();
    let cleanup = wlmb_server::native_host::NativeHost::new(
        Arc::new(NativeBus::default()),
        wlmb_server::private_store::PrivateStore::open(std::path::Path::new("/etc/wlmb-server"))
            .unwrap(),
    );
    wlmb_server::host_controls::HostIo::clear_guard(cleanup.as_ref()).unwrap();
    let lan = service::lan(Arc::new(NativeBus::default()).as_ref()).unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        Command::new(binary)
            .arg("--serve-readonly")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect_timeout(&(lan.address, 1117).into(), Duration::from_millis(100))
        .is_err()
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    let request = |path: &str, body: Option<serde_json::Value>, token: Option<&str>| {
        let mut stream = TcpStream::connect((lan.address, 1117)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let raw = body.as_ref().map_or(String::new(), |v| v.to_string());
        let request = format!(
            "{} {path} HTTP/1.1\r\nHost: {}:1117\r\nConnection: close\r\n{}{}\r\n{raw}",
            if body.is_some() { "POST" } else { "GET" },
            lan.address,
            if body.is_some() {
                format!(
                    "Content-Type: application/json\r\nContent-Length: {}\r\n",
                    raw.len()
                )
            } else {
                String::new()
            },
            token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"))
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut reply = String::new();
        if let Err(error) = stream.read_to_string(&mut reply) {
            panic!(
                "Native HTTP {path} read failed: {error}; received {} bytes",
                reply.len()
            );
        }
        let (head, body) = reply.split_once("\r\n\r\n").unwrap();
        (
            head.split(' ').nth(1).unwrap().parse::<u16>().unwrap(),
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
        )
    };
    assert_eq!(request("/health", None, None).1["data"]["backend"], "rust");
    assert_eq!(request("/api/status", Some(json!({})), None).0, 401);
    let login = request(
        "/api/login",
        Some(json!({"username":"root","password":"NetworkPanelLocalTestOnly"})),
        None,
    );
    assert_eq!(login.0, 200);
    let token = login.1["data"]["token"].as_str().unwrap();
    let mut status = request("/api/status", Some(json!({})), Some(token));
    while status.1["data"]["system"]["board"].is_null() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
        status = request("/api/status", Some(json!({})), Some(token));
    }
    assert_eq!(status.0, 200);
    assert!(status.1["data"]["system"]["board"]["model"].is_string());
    assert!(status.1["data"]["modems"].as_array().unwrap().is_empty());
    let receipt = request("/api/lock_execute", Some(json!({})), Some(token));
    assert_eq!(receipt.0, 200);
    let job = receipt.1["data"]["job"].as_str().unwrap();
    let mut result = request("/api/control_result", Some(json!({"job":job})), Some(token));
    while result.1["data"]["state"] == "pending" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
        result = request("/api/control_result", Some(json!({"job":job})), Some(token));
    }
    assert_eq!(result.1["data"]["state"], "error");
    assert_eq!(
        request("/api/diagnostics", Some(json!({})), Some(token)).1["data"]["service"],
        "wlmb-server"
    );
    assert_eq!(request("/api/logout", Some(json!({})), Some(token)).0, 200);
    assert_eq!(request("/api/status", Some(json!({})), Some(token)).0, 401);
    let before = Command::new("nft")
        .args(["list", "table", "inet", "fw4"])
        .output()
        .unwrap();
    assert!(before.status.success());
    assert!(
        Command::new("nft")
            .args([
                "add",
                "table",
                "inet",
                "wlmb_guard",
                "{ comment \"wlmb-server/1\"; }"
            ])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let until = Instant::now() + Duration::from_secs(12);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < until,
            "SIGTERM did not finish bounded graceful cleanup"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(std::path::Path::new("/etc/wlmb-server/traffic.json").is_file());
    assert!(
        !Command::new("nft")
            .args(["list", "table", "inet", "wlmb_guard"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let after = Command::new("nft")
        .args(["list", "table", "inet", "fw4"])
        .output()
        .unwrap();
    assert!(after.status.success());
    assert_eq!(before.stdout, after.stdout);
}
