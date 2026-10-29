//! fnOS host adapter: existing radio sampler/parser, private local read-only RPC.
//! LicenseRef-WLMB-NonCommercial. No OpenWrt UCI/ubus or dial-up ownership.
use crate::{
    PanelError, Result,
    independent_at::{Endpoint, Hardware, IndependentSource, SystemHardware},
    sampler::Sampler,
    service::NativeService,
    transport::{Bus, Discovery, SystemDiscovery},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

fn unavailable() -> PanelError {
    PanelError::new("FNOS_SYSTEM_UNAVAILABLE", 503)
}
struct FnosHardware;
struct FnosEndpoint(Box<dyn Endpoint>);
impl Endpoint for FnosEndpoint {
    fn synchronize(&mut self) -> Result<()> {
        self.0.synchronize()
    }
    fn quarantined(&self) -> bool {
        self.0.quarantined()
    }
    fn send_pdu(&mut self, _: &str, _: usize) -> Result<String> {
        Err(PanelError::new("FEATURE_NOT_READY", 501))
    }
    fn query(&mut self, command: &str, timeout: Duration) -> Result<String> {
        let answer = self.0.query(command, timeout)?;
        if command == "AT+CGMM" && answer.contains("ERROR") {
            // Some RM500U firmware accepts ATI but rejects CGMM.
            self.0.query("ATI", timeout)
        } else {
            Ok(answer)
        }
    }
}
impl Hardware for FnosHardware {
    fn inventory(&self) -> Result<Vec<Value>> {
        let rows = crate::modem_inventory::scan_sysfs(Path::new("/sys"))?;
        let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for row in rows {
            groups
                .entry(row["physicalDevice"].as_str().unwrap_or("").to_owned())
                .or_default()
                .push(row);
        }
        let mut result = vec![];
        for (physical, group) in groups {
            let mut candidates: Vec<_> = group
                .iter()
                .filter(|row| crate::modem_inventory::at_candidate(row))
                .cloned()
                .collect();
            if candidates.is_empty() {
                // Verified RM500U-CNV NCM layout: interface 04 is the primary AT port.
                // USB device and interface identities are checked, not ttyUSB numbering.
                candidates = group
                    .into_iter()
                    .filter(|row| {
                        if row["vendorId"] != "2c7c" || row["productId"] != "0900" {
                            return false;
                        }
                        let Some(port) = row["port"].as_str() else {
                            return false;
                        };
                        let Some(name) = Path::new(port).file_name() else {
                            return false;
                        };
                        let Ok(path) =
                            fs::canonicalize(Path::new("/sys/class/tty").join(name).join("device"))
                        else {
                            return false;
                        };
                        path.ancestors().take(4).any(|parent| {
                            fs::read_to_string(parent.join("bInterfaceNumber"))
                                .is_ok_and(|value| value.trim() == "04")
                        })
                    })
                    .collect();
            }
            if candidates.len() != 1 {
                continue;
            }
            let mut row = candidates.remove(0);
            let port = row["port"].as_str().ok_or_else(unavailable)?;
            let meta = fs::metadata(port).map_err(|_| unavailable())?;
            use sha2::{Digest, Sha256};
            row["id"] = json!(format!("fnos-{:x}", Sha256::digest(physical.as_bytes())));
            row["model"] = json!("unknown");
            row["instance"] = json!(format!("{}:{}", meta.rdev(), meta.ino()));
            result.push(row);
        }
        Ok(result)
    }
    fn owners(&self, port: &str) -> Result<Vec<String>> {
        SystemDiscovery::default().owners(port)
    }
    fn open(&self, port: &str) -> Result<Box<dyn Endpoint>> {
        Ok(Box::new(FnosEndpoint(SystemHardware.open(port)?)))
    }
}

struct FnosBus;
impl Bus for FnosBus {
    fn call(&self, object: &str, method: &str, _: Value, _: Duration) -> Result<Value> {
        if object == "system" && method == "board" {
            let model = fs::read_to_string("/proc/device-tree/model")
                .unwrap_or_else(|_| "fnOS".into())
                .trim_matches('\0')
                .trim()
                .to_owned();
            return Ok(json!({"model":model,"system":"fnOS","release":{"distribution":"fnOS"}}));
        }
        if object == "network.interface" && method == "dump" {
            let output = Command::new("/usr/sbin/ip")
                .args(["-j", "-4", "route", "show", "default"])
                .output()
                .map_err(|_| unavailable())?;
            let routes: Vec<Value> =
                serde_json::from_slice(&output.stdout).map_err(|_| unavailable())?;
            let selected = routes
                .iter()
                .filter(|v| v["dev"].is_string())
                .min_by_key(|v| v["metric"].as_u64().unwrap_or(0));
            return Ok(
                json!({"interface":selected.map(|v|json!({"interface":"wan","l3_device":v["dev"],"up":true,"route":[{"target":"0.0.0.0","mask":0,"metric":v["metric"]}]})).into_iter().collect::<Vec<_>>()}),
            );
        }
        Err(PanelError::new("FNOS_FEATURE_UNAVAILABLE", 501))
    }
}

fn authenticate(auth: &Path, request: &Value) -> Result<Value> {
    let mut child = Command::new("/usr/bin/python3")
        .arg(auth)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| unavailable())?;
    if let Some(mut input) = child.stdin.take() {
        input
            .write_all(
                serde_json::to_string(request)
                    .map_err(|_| unavailable())?
                    .as_bytes(),
            )
            .map_err(|_| unavailable())?;
    }
    let output = child.wait_with_output().map_err(|_| unavailable())?;
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| unavailable())?;
    if output.status.success() && value["ok"] == true {
        Ok(json!({"verified":true}))
    } else {
        Err(PanelError::new("LOGIN_FAILED", 401))
    }
}

fn answer(mut channel: UnixStream, service: &Arc<NativeService>, auth: &Path) -> Result<()> {
    channel
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| unavailable())?;
    channel
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| unavailable())?;
    let mut raw = vec![];
    BufReader::new(&mut channel)
        .take(16385)
        .read_until(b'\n', &mut raw)
        .map_err(|_| unavailable())?;
    if raw.len() > 16384 || !raw.ends_with(b"\n") {
        return Err(unavailable());
    }
    let request: Value = serde_json::from_slice(&raw).map_err(|_| unavailable())?;
    let operation = request["operation"].as_str().unwrap_or("").to_owned();
    let result = match operation.as_str() {
        "authenticate" => authenticate(auth, &request),
        "status" | "settings" | "diagnostics" => service.local(&operation, request),
        "refresh" => service
            .sampler
            .refresh(request["modem"].as_str().unwrap_or("")),
        "health" => Ok(json!({"platform":"fnOS","version":"0.1.0","backend":"rust"})),
        _ => Err(PanelError::new("FEATURE_NOT_READY", 501)),
    };
    let response = match result {
        Ok(data) => json!({"ok":true,"data":data}),
        Err(e) => json!({"ok":false,"error":e.details(),"status":e.status}),
    };
    let data = serde_json::to_vec(&response).map_err(|_| unavailable())?;
    if data.len() > 262144 {
        return Err(unavailable());
    }
    channel.write_all(&data).map_err(|_| unavailable())?;
    Ok(())
}

pub fn run() -> Result<()> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() != 2 {
        return Err(PanelError::new("FNOS_EXPECT_SOCKET_AND_AUTH_HELPER", 422));
    }
    let socket = PathBuf::from(&arguments[0]);
    let auth = PathBuf::from(&arguments[1]);
    let source = IndependentSource::new(Arc::new(FnosHardware));
    let sampler = Sampler::new(source);
    let service = NativeService::new(Arc::new(FnosBus), sampler.clone());
    let stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone())
        .map_err(|_| unavailable())?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())
        .map_err(|_| unavailable())?;
    let sample_worker = {
        let sampler = sampler.clone();
        std::thread::spawn(move || sampler.run())
    };
    let host_worker = service.sample_host(stop.clone());
    if socket.exists() {
        fs::remove_file(&socket).map_err(|_| unavailable())?;
    }
    let listener = UnixListener::bind(&socket).map_err(|_| unavailable())?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o660)).map_err(|_| unavailable())?;
    listener.set_nonblocking(true).map_err(|_| unavailable())?;
    let active = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((channel, _)) => {
                if active.load(Ordering::Relaxed) >= 8 {
                    continue;
                }
                active.fetch_add(1, Ordering::Relaxed);
                let (service, auth, active) = (service.clone(), auth.clone(), active.clone());
                std::thread::spawn(move || {
                    let _ = answer(channel, &service, &auth);
                    active.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(_) => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = sampler.stop(Duration::from_secs(15));
    let _ = sample_worker.join();
    let _ = host_worker.join();
    let _ = fs::remove_file(socket);
    Ok(())
}
