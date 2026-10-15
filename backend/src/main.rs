//! 服务、RPC、型号目录及离线解析入口，按启动参数分派。
use std::io::{self, BufRead, Read, Write};
use wlmb_server::{MAX_AT_RESPONSE, radio};

fn main() {
    if std::env::args().skip(1).collect::<Vec<_>>() == ["--catalog"] {
        println!("{}", wlmb_server::catalog::inventory());
        return;
    }
    if std::env::args().skip(1).collect::<Vec<_>>() == ["--version"] {
        println!("{}", wlmb_server::BUILD_MARKER);
        return;
    }
    #[cfg(unix)]
    if std::env::args().skip(1).collect::<Vec<_>>() == ["--serve-auto"] {
        if let Err(error) = wlmb_server::service::run_auto() {
            eprintln!("wlmb-server: {}", error.code);
            std::process::exit(1);
        }
        return;
    }
    #[cfg(unix)]
    if std::env::args().skip(1).collect::<Vec<_>>() == ["--serve-standalone"] {
        if let Err(error) = wlmb_server::service::run_standalone() {
            eprintln!("wlmb-server: {}", error.code);
            std::process::exit(1);
        }
        return;
    }
    #[cfg(unix)]
    if std::env::args()
        .next()
        .as_deref()
        .is_some_and(|name| name.ends_with("/network-panel"))
    {
        if let Err(error) = wlmb_server::rpc::cli() {
            eprintln!("wlmb-server: {}", error.code);
            std::process::exit(1);
        }
        return;
    }
    #[cfg(unix)]
    if std::env::args().skip(1).collect::<Vec<_>>() == ["--serve-readonly"] {
        if let Err(error) = wlmb_server::service::run_readonly() {
            eprintln!("wlmb-server: {}", error.code);
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--offline"] {
        eprintln!(
            "wlmb-server: use --serve-auto, --serve-standalone for owned AT, --serve-readonly for explicit shared mode, or --offline for fixtures"
        );
        std::process::exit(2);
    }
    let mut input = io::stdin().lock();
    let mut output = io::BufWriter::new(io::stdout().lock());
    loop {
        let mut line = Vec::new();
        match (&mut input)
            .take((MAX_AT_RESPONSE * 6 + 1024) as u64)
            .read_until(b'\n', &mut line)
        {
            Ok(0) => break,
            Ok(_) if line.ends_with(b"\n") => {}
            _ => {
                eprintln!("wlmb-server: invalid offline input frame");
                std::process::exit(2);
            }
        }
        let request: serde_json::Value = match serde_json::from_slice(&line) {
            Ok(value) => value,
            Err(_) => {
                eprintln!("wlmb-server: invalid offline JSON");
                std::process::exit(2);
            }
        };
        let parser = request["parser"].as_str().unwrap_or("");
        let raw = request["raw"].as_str().unwrap_or("");
        let result = match request["operation"].as_str() {
            Some("lock_command") => wlmb_server::controls::lock_command(&request["value"])
                .map(serde_json::Value::String),
            Some("parse_lock") => {
                wlmb_server::controls::parse_lock(raw, request["rat"].as_str().unwrap_or(""))
            }
            Some("read_only") => Ok(serde_json::json!(wlmb_server::security::read_only(raw))),
            Some("identify") => Ok(wlmb_server::catalog::identify(raw)),
            Some("cpu_times") => Ok(serde_json::json!(wlmb_server::telemetry::cpu_times(raw))),
            Some("memory") => {
                Ok(wlmb_server::telemetry::memory(raw).unwrap_or(serde_json::Value::Null))
            }
            Some("wan_device") => Ok(serde_json::json!(wlmb_server::telemetry::wan_device(
                &request["value"]
            ))),
            _ => radio::parse(parser, raw),
        };
        let answer = match result {
            Ok(data) => serde_json::json!({"ok": true, "data": data}),
            Err(error) => serde_json::json!({"ok": false, "error": {"code":error.code}}),
        };
        if serde_json::to_writer(&mut output, &answer).is_err()
            || output.write_all(b"\n").is_err()
            || output.flush().is_err()
        {
            std::process::exit(2);
        }
    }
}
