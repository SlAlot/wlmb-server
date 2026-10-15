//! 共享通道的有界调用；参考QModem公开IPC协议，不抢占串口，不重放未知写入。
use crate::{MAX_AT_RESPONSE, PanelError, Result, security};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    os::{
        fd::OwnedFd,
        unix::{
            fs::{FileTypeExt, MetadataExt},
            net::UnixStream,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub trait Bus: Send + Sync {
    fn call(&self, object: &str, method: &str, params: Value, timeout: Duration) -> Result<Value>;
    fn describe(&self, _object: &str, _timeout: Duration) -> Result<Value> {
        Err(PanelError::new("SHARED_TRANSACTIONS_UNAVAILABLE", 422))
    }
    fn sms_events(&self) -> Result<Box<dyn SmsEvents>> {
        Err(PanelError::new("SMS_EVENTS_UNAVAILABLE", 422))
    }
    fn qmodem_sms_events(&self) -> Result<Box<dyn SmsEvents>> {
        Err(PanelError::new("SMS_EVENTS_UNAVAILABLE", 422))
    }
}
pub trait SmsEvents: Send {
    fn next(&mut self, timeout: Duration) -> Result<Option<Value>>;
}
fn unavailable() -> PanelError {
    PanelError::new("SYSTEM_UNAVAILABLE", 503)
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(unavailable)
}
fn read_exact(channel: &mut UnixStream, raw: &mut [u8], deadline: Instant) -> Result<()> {
    let mut offset = 0;
    while offset < raw.len() {
        channel
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|_| unavailable())?;
        let count = channel
            .read(&mut raw[offset..])
            .map_err(|_| unavailable())?;
        if count == 0 {
            return Err(unavailable());
        }
        offset += count;
    }
    Ok(())
}
fn write_all(channel: &mut UnixStream, raw: &[u8], deadline: Instant) -> Result<()> {
    let mut offset = 0;
    while offset < raw.len() {
        channel
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(|_| unavailable())?;
        let count = channel.write(&raw[offset..]).map_err(|_| unavailable())?;
        if count == 0 {
            return Err(unavailable());
        }
        offset += count;
    }
    Ok(())
}
pub fn read_frame(channel: &mut UnixStream, deadline: Instant) -> Result<Value> {
    let mut header = [0; 8];
    read_exact(channel, &mut header, deadline)?;
    if !header
        .iter()
        .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(unavailable());
    }
    let length =
        usize::from_str_radix(std::str::from_utf8(&header).map_err(|_| unavailable())?, 16)
            .map_err(|_| unavailable())?;
    if !(1..=262144).contains(&length) {
        return Err(unavailable());
    }
    let mut raw = vec![0; length];
    read_exact(channel, &mut raw, deadline)?;
    serde_json::from_slice(&raw).map_err(|_| unavailable())
}
pub fn write_frame(channel: &mut UnixStream, value: &Value, deadline: Instant) -> Result<()> {
    let raw = serde_json::to_vec(value).map_err(|_| unavailable())?;
    if raw.is_empty() || raw.len() > 32768 {
        return Err(unavailable());
    }
    write_all(channel, format!("{:08x}", raw.len()).as_bytes(), deadline)?;
    write_all(channel, &raw, deadline)
}
struct Helper {
    child: Child,
    channel: UnixStream,
}
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.channel.shutdown(std::net::Shutdown::Both);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub type NativeBus = crate::native_ubus::NativeBus;

pub(crate) fn allowed(object: &str, method: &str) -> bool {
    match object {
        "session" => ["login", "access", "destroy"].contains(&method),
        "at-daemon" => ["list", "sendat", "lease_acquire", "lease_release"].contains(&method),
        "qmodem.sms" => [
            "status",
            "config_get",
            "sync",
            "list",
            "get",
            "send_managed",
            "delete",
        ]
        .contains(&method),
        "system" => method == "board",
        "luci" => method == "getFeatures",
        "network.interface" => method == "dump",
        "network.wireless" => method == "status",
        "network" => method == "reload",
        "network.interface.lan" => method == "status",
        "uci" => method == "get",
        "iwinfo" => ["info", "assoclist", "freqlist"].contains(&method),
        _ => false,
    }
}

#[cfg(test)]
mod feature_allowlist_tests {
    #[test]
    fn wifi_features_are_readable_without_general_luci_access() {
        assert!(super::allowed("luci", "getFeatures"));
        assert!(!super::allowed("luci", "setFeatures"));
        assert!(!super::allowed("luci", "exec"));
    }
    #[test]
    fn sms_database_allowlist_cannot_reconfigure_or_claim_other_forwarder_deliveries() {
        for method in [
            "status",
            "config_get",
            "sync",
            "list",
            "get",
            "send_managed",
            "delete",
        ] {
            assert!(super::allowed("qmodem.sms", method));
        }
        for method in [
            "send",
            "configure",
            "storage_set",
            "modem_delete",
            "mark_read",
            "delivery_claim",
            "delivery_complete",
        ] {
            assert!(!super::allowed("qmodem.sms", method));
        }
    }
}
pub fn process(
    program: &str,
    args: &[&str],
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<String> {
    if ![
        "/sbin/uci",
        "/sbin/nft",
        "/usr/sbin/nft",
        "/usr/bin/ip",
        "/sbin/ip",
        "/sbin/wifi",
        "/etc/init.d/wlmb-server",
        "/bin/ubus",
        "/bin/date",
    ]
    .contains(&program)
        || timeout.is_zero()
        || timeout > Duration::from_secs(30)
    {
        return Err(unavailable());
    }
    let deadline = Instant::now() + timeout;
    let (mut parent, child) = UnixStream::pair().map_err(|_| unavailable())?;
    let output = Stdio::from(OwnedFd::from(child.try_clone().map_err(|_| unavailable())?));
    let input_fd = if input.is_some() {
        Stdio::from(OwnedFd::from(child))
    } else {
        drop(child);
        Stdio::null()
    };
    let child = Command::new(program)
        .args(args)
        .stdin(input_fd)
        .stdout(output)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| unavailable())?;
    let mut helper = Helper {
        child,
        channel: parent.try_clone().map_err(|_| unavailable())?,
    };
    if let Some(payload) = input {
        if payload.len() > 32768 {
            return Err(unavailable());
        }
        write_all(&mut parent, payload, deadline)?;
        parent
            .shutdown(std::net::Shutdown::Write)
            .map_err(|_| unavailable())?;
    }
    let mut raw = vec![];
    let mut chunk = [0; 8192];
    loop {
        parent
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|_| unavailable())?;
        let count = parent.read(&mut chunk).map_err(|_| unavailable())?;
        if count == 0 {
            break;
        }
        if raw.len() + count > 262144 {
            return Err(unavailable());
        }
        raw.extend_from_slice(&chunk[..count]);
    }
    loop {
        match helper.child.try_wait() {
            Ok(Some(exit)) => {
                if !exit.success() {
                    return Err(unavailable());
                }
                break;
            }
            Ok(None) => {
                remaining(deadline)?;
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return Err(unavailable()),
        }
    }
    Ok(String::from_utf8_lossy(&raw).into_owned())
}
fn identifier(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 128
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn valid_port(v: &str) -> bool {
    crate::modem_inventory::valid_port(v)
}
fn unquote(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for c in raw.trim().chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\'' {
            quoted = !quoted;
        } else if c == '\\' && !quoted {
            escaped = true;
        } else if c.is_whitespace() && !quoted {
            return None;
        } else {
            out.push(c);
        }
    }
    (!quoted && !escaped).then_some(out)
}
pub fn parse_config(raw: &str) -> Vec<Value> {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut order = vec![];
    for line in raw.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(key) = key.strip_prefix("qmodem.") else {
            continue;
        };
        let (section, key) = key.split_once('.').unwrap_or((key, ".type"));
        if !identifier(section) || key != ".type" && !identifier(key) {
            continue;
        }
        let Some(value) = unquote(value) else {
            continue;
        };
        if !sections.contains_key(section) {
            if sections.len() >= 64 {
                continue;
            }
            order.push(section.to_owned());
        }
        sections
            .entry(section.to_owned())
            .or_default()
            .insert(key.to_owned(), value);
    }
    let main = sections.get("main");
    let tool = main
        .and_then(|m| m.get("at_tool"))
        .map(String::as_str)
        .unwrap_or("0");
    let mut rows = vec![];
    for name in order {
        if name == "main" {
            continue;
        }
        let Some(s) = sections.get(&name) else {
            continue;
        };
        let port = s
            .get("override_at_port")
            .filter(|s| !s.is_empty())
            .or_else(|| s.get("at_port"))
            .map(String::as_str)
            .unwrap_or("");
        if !valid_port(port) {
            continue;
        }
        let model = s
            .get("name")
            .filter(|s| !s.is_empty())
            .or_else(|| s.get("model"))
            .map(String::as_str)
            .unwrap_or("unknown");
        let sms_port = s
            .get("override_at_port")
            .filter(|v| !v.is_empty())
            .or_else(|| s.get("sms_at_port").filter(|v| !v.is_empty()))
            .map(String::as_str)
            .unwrap_or(port);
        rows.push(json!({"id":name,"port":port,"smsPort":sms_port,"model":model,"manufacturer":s.get("manufacturer").cloned().unwrap_or_default(),"platform":s.get("platform").cloned().unwrap_or_default(),"shared":s.get("use_ubus").is_some_and(|v|v=="1")&&tool!="1","discoveredBy":"qmodem-config","profile":crate::catalog::identify(model)}));
    }
    rows
}
pub trait Discovery: Send + Sync {
    fn modems(&self) -> Result<Vec<Value>>;
    fn owners(&self, port: &str) -> Result<Vec<String>>;
    fn shared_queue(&self) -> Option<Arc<crate::qmodem_queue::Contract>> {
        None
    }
}
pub struct SystemDiscovery {
    pub proc_root: PathBuf,
}
impl Default for SystemDiscovery {
    fn default() -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
        }
    }
}
impl Discovery for SystemDiscovery {
    fn shared_queue(&self) -> Option<Arc<crate::qmodem_queue::Contract>> {
        crate::qmodem_queue::Contract::installed().map(Arc::new)
    }
    fn modems(&self) -> Result<Vec<Value>> {
        match fs::metadata("/etc/config/qmodem") {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(_) => return Err(unavailable()),
            Ok(metadata) if !metadata.is_file() => return Err(unavailable()),
            Ok(_) => {}
        }
        let mut rows = parse_config(&process(
            "/sbin/uci",
            &["-q", "show", "qmodem"],
            None,
            Duration::from_secs(3),
        )?);
        for row in &mut rows {
            let port = row["port"].as_str().ok_or_else(unavailable)?;
            if let Ok(meta) = fs::metadata(port) {
                if meta.file_type().is_char_device() {
                    row["instance"] = json!(format!("{}:{}", meta.rdev(), meta.ino()));
                }
            }
        }
        Ok(rows)
    }
    fn owners(&self, port: &str) -> Result<Vec<String>> {
        let error = || PanelError::new("AT_OWNERSHIP_UNKNOWN", 409);
        if !valid_port(port) {
            return Err(error());
        }
        let target = fs::metadata(port).map_err(|_| error())?;
        if !target.file_type().is_char_device() {
            return Err(error());
        }
        let mut owners = vec![];
        let deadline = Instant::now() + Duration::from_secs(2);
        for (index, task) in fs::read_dir(&self.proc_root)
            .map_err(|_| error())?
            .enumerate()
        {
            if index >= 8192 || Instant::now() >= deadline {
                return Err(error());
            }
            let task = task.map_err(|_| error())?;
            let name = task.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if name.parse::<u32>().ok() == Some(std::process::id()) {
                continue;
            }
            let executable = match fs::read_link(task.path().join("exe")) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(error()),
            };
            let descriptors = match fs::read_dir(task.path().join("fd")) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(error()),
            };
            for (index, fd) in descriptors.enumerate() {
                if index >= 8192 || Instant::now() >= deadline {
                    return Err(error());
                }
                let fd = fd.map_err(|_| error())?;
                let meta = match fs::metadata(fd.path()) {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return Err(error()),
                };
                if meta.file_type().is_char_device() && meta.rdev() == target.rdev() {
                    owners.push(
                        executable
                            .file_name()
                            .and_then(|s| s.to_str())
                            .ok_or_else(error)?
                            .to_owned(),
                    );
                    break;
                }
            }
        }
        Ok(owners)
    }
}
#[derive(Default)]
struct Ports {
    locks: HashMap<String, Arc<Mutex<()>>>,
    cooldown: HashMap<String, Instant>,
    quarantined: HashMap<String, Value>,
    retained: HashMap<String, Arc<crate::qmodem_queue::Held>>,
}
pub struct Broker {
    pub bus: Arc<dyn Bus>,
    pub discovery: Arc<dyn Discovery>,
    ports: Mutex<Ports>,
    pub notices: crate::sms_notice::Notices,
    sms_writable: bool,
    pub(crate) sms_leased: bool,
    pub(crate) sms_events: bool,
    queue: Option<Arc<crate::qmodem_queue::Contract>>,
}
impl Broker {
    pub fn new(bus: Arc<dyn Bus>, discovery: Arc<dyn Discovery>) -> Self {
        let schema = bus
            .describe("at-daemon", Duration::from_secs(3))
            .unwrap_or(Value::Null);
        let queue = discovery.shared_queue();
        let sms_leased = crate::shared_sms::supported(&schema);
        let sms_events = schema["urc_list"].is_object()
            && [("at_port", 3), ("owner", 3), ("urc_id", 3), ("prefix", 3)]
                .iter()
                .all(|(key, kind)| schema["urc_register"][*key] == *kind);
        let sms_writable =
            sms_leased || (queue.is_some() && crate::shared_sms::raw_supported(&schema));
        Self {
            bus,
            discovery,
            ports: Mutex::new(Ports::default()),
            notices: crate::sms_notice::Notices::default(),
            sms_writable,
            sms_leased,
            sms_events,
            queue,
        }
    }
    pub fn modems(&self) -> Result<Vec<Value>> {
        let mut rows = self.discovery.modems()?;
        let mut ports = self.ports.lock().map_err(|_| unavailable())?;
        let now = Instant::now();
        ports.quarantined.retain(|port, instance| {
            rows.iter()
                .any(|r| r["port"] == port.as_str() && r["instance"] == *instance)
        });
        let quarantined = ports.quarantined.keys().cloned().collect::<Vec<_>>();
        ports.retained.retain(|port, _| quarantined.contains(port));
        for row in &mut rows {
            if row["port"]
                .as_str()
                .is_some_and(|port| ports.quarantined.contains_key(port))
            {
                row["channelState"] = json!("sms-input-unknown");
                row["channelReady"] = json!(false);
            }
        }
        ports.locks.retain(|port, lock| {
            rows.iter().any(|r| r["port"] == port.as_str()) || lock.try_lock().is_err()
        });
        let active = ports.locks.keys().cloned().collect::<Vec<_>>();
        ports.cooldown.retain(|port, until| {
            *until > now
                && (active.contains(port) || rows.iter().any(|r| r["port"] == port.as_str()))
        });
        Ok(rows)
    }
    pub fn query(&self, id: &str, command: &str, expected: Option<(&str, &str)>) -> Result<String> {
        let seconds = if matches!(command, "AT+CMGL=4" | "AT+CMGL=4,1" | "AT+CMGL=\"ALL\"") {
            10
        } else {
            3
        };
        self.query_checked(id, command, expected, None, seconds, None)
    }
    pub fn query_bound(&self, row: &Value, command: &str, seconds: u64) -> Result<String> {
        self.query_checked(
            row["id"].as_str().ok_or_else(unavailable)?,
            command,
            Some((
                row["port"].as_str().ok_or_else(unavailable)?,
                row["model"].as_str().ok_or_else(unavailable)?,
            )),
            Some(&row["instance"]),
            seconds,
            None,
        )
    }
    pub fn sms_write_supported(&self) -> bool {
        self.sms_writable && self.queue.as_ref().is_none_or(|q| q.validate().is_ok())
    }
    pub(crate) fn queue_hold(&self, port: &str) -> Result<Option<Arc<crate::qmodem_queue::Held>>> {
        self.queue
            .as_ref()
            .map(|q| q.acquire(port).map(Arc::new))
            .transpose()
    }
    pub(crate) fn retain_queue(&self, row: &Value, held: Arc<crate::qmodem_queue::Held>) {
        if let (Some(port), Ok(mut state)) = (row["port"].as_str(), self.ports.lock()) {
            state.retained.insert(port.into(), held);
        }
    }
    pub(crate) fn sms_quarantine(&self, row: &Value) {
        if let (Some(port), Ok(mut state)) = (row["port"].as_str(), self.ports.lock()) {
            state
                .quarantined
                .insert(port.into(), row["instance"].clone());
        }
    }
    pub(crate) fn sms_transaction<T>(
        &self,
        row: &Value,
        action: impl FnOnce(&mut crate::shared_sms::Lease<'_>) -> Result<T>,
    ) -> Result<T> {
        if !self.sms_write_supported() {
            return Err(PanelError::new("SMS_SHARED_READ_ONLY", 422));
        }
        let port = row["port"].as_str().ok_or_else(unavailable)?;
        if !self.modems()?.iter().any(|r| {
            r["id"] == row["id"]
                && r["port"] == row["port"]
                && r["model"] == row["model"]
                && r["instance"] == row["instance"]
                && r["shared"] == true
        }) {
            return Err(PanelError::new("MODEM_CHANGED", 409));
        }
        let owners = self.discovery.owners(port)?;
        if owners.iter().any(|owner| owner != "ubus-at-daemon") || owners.is_empty() {
            return Err(PanelError::new("AT_PORT_CONFLICT", 409));
        }
        let lock = {
            let mut state = self.ports.lock().map_err(|_| unavailable())?;
            if state.quarantined.contains_key(port) {
                return Err(PanelError::new("SMS_INPUT_UNKNOWN", 409));
            }
            if state
                .cooldown
                .get(port)
                .is_some_and(|at| *at > Instant::now())
            {
                return Err(PanelError::new("AT_COOLDOWN", 503));
            }
            state.locks.entry(port.into()).or_default().clone()
        };
        let _held = lock
            .try_lock()
            .map_err(|_| PanelError::new("AT_BUSY", 409))?;
        let mut lease = crate::shared_sms::Lease::acquire(self, row)?;
        action(&mut lease)
    }
    pub(crate) fn sms_query(
        &self,
        lease: &crate::shared_sms::Lease<'_>,
        command: &str,
        payload: Option<&str>,
        seconds: u64,
        end_flag: Option<&str>,
    ) -> Result<String> {
        if let Some(held) = &lease.held {
            held.validate()?;
        }
        if lease.deadline.saturating_duration_since(Instant::now())
            < Duration::from_secs(seconds + 5)
        {
            return Err(PanelError::new("SMS_TRANSACTION_TIMEOUT", 503));
        }
        self.query_checked(
            lease.row["id"].as_str().ok_or_else(unavailable)?,
            command,
            Some((
                lease.row["port"].as_str().ok_or_else(unavailable)?,
                lease.row["model"].as_str().ok_or_else(unavailable)?,
            )),
            Some(&lease.row["instance"]),
            seconds,
            Some((&lease.owner, payload, end_flag)),
        )
    }
    fn query_checked(
        &self,
        id: &str,
        command: &str,
        expected: Option<(&str, &str)>,
        instance: Option<&Value>,
        seconds: u64,
        shared: Option<(&str, Option<&str>, Option<&str>)>,
    ) -> Result<String> {
        security::validate_at(command)?;
        if !(1..=10).contains(&seconds) {
            return Err(unavailable());
        }
        let rows = self.modems()?;
        let row = rows
            .iter()
            .find(|r| r["id"] == id)
            .ok_or_else(|| PanelError::new("MODEM_NOT_FOUND", 404))?;
        let port = row["port"].as_str().ok_or_else(unavailable)?;
        let model = row["model"].as_str().ok_or_else(unavailable)?;
        if expected.is_some_and(|signature| signature != (port, model)) {
            return Err(PanelError::new("MODEM_CHANGED", 409));
        }
        if instance.is_some_and(|value| value != &row["instance"]) {
            return Err(PanelError::new("MODEM_CHANGED", 409));
        }
        if row["shared"] != true {
            return Err(PanelError::new("AT_NOT_SHARED", 409));
        }
        let lock = {
            let mut state = self.ports.lock().map_err(|_| unavailable())?;
            if shared.is_none() && state.quarantined.contains_key(port) {
                return Err(PanelError::new("SMS_INPUT_UNKNOWN", 409));
            }
            if shared.is_none()
                && state
                    .cooldown
                    .get(port)
                    .is_some_and(|until| *until > Instant::now())
            {
                return Err(PanelError::new("AT_COOLDOWN", 503));
            }
            state.locks.entry(port.to_owned()).or_default().clone()
        };
        let _held = if shared.is_none() {
            Some(
                lock.try_lock()
                    .map_err(|_| PanelError::new("AT_BUSY", 409))?,
            )
        } else {
            None
        };
        let queue_held = if shared.is_none() {
            self.queue_hold(port)?
        } else {
            None
        };
        if rows
            .iter()
            .any(|r| r["port"] == port && r["shared"] != true)
        {
            return Err(PanelError::new("AT_PORT_CONFLICT", 409));
        }
        let answer = self
            .bus
            .call("at-daemon", "list", json!({}), Duration::from_secs(3))?;
        if !answer["ports"].as_array().is_some_and(|list| {
            list.iter()
                .any(|r| r["port"] == port && r["is_open"].as_u64() == Some(1))
        }) {
            return Err(PanelError::new("AT_NOT_READY", 409));
        }
        let owners = self.discovery.owners(port)?;
        if owners.is_empty() || owners.iter().any(|name| name != "ubus-at-daemon") {
            return Err(PanelError::new("AT_PORT_CONFLICT", 409));
        }
        let unchanged = || -> Result<()> {
            if let Some(queue) = &self.queue {
                queue.validate()?;
            }
            if let Some(held) = &queue_held {
                held.validate()?;
            }
            if instance.is_some()
                && !self.discovery.modems()?.iter().any(|r| {
                    r["id"] == id
                        && r["port"] == port
                        && r["model"] == model
                        && r["instance"] == row["instance"]
                        && r["shared"] == true
                })
            {
                return Err(PanelError::new("MODEM_CHANGED", 409));
            }
            Ok(())
        };
        unchanged()?;
        let mut params = json!({"at_port":port,"at_cmd":command,"timeout":seconds});
        if let Some((owner, payload, end_flag)) = shared {
            if !owner.is_empty() {
                params["owner"] = json!(owner);
            }
            if let Some(payload) = payload {
                params
                    .as_object_mut()
                    .ok_or_else(unavailable)?
                    .remove("at_cmd");
                params["raw_at_content"] = json!(crate::shared_sms::wire_raw(payload)?);
            }
            if let Some(end_flag) = end_flag {
                params["end_flag"] = json!(end_flag);
            }
        }
        let result = self.bus.call(
            "at-daemon",
            "sendat",
            params,
            Duration::from_secs(seconds + 2),
        );
        let result = match result {
            Ok(v) => v,
            Err(e) => {
                if let Ok(mut s) = self.ports.lock() {
                    s.cooldown
                        .insert(port.to_owned(), Instant::now() + Duration::from_secs(15));
                }
                return Err(e);
            }
        };
        let raw = result["response"]
            .as_str()
            .filter(|r| r.len() <= MAX_AT_RESPONSE);
        if result["status"] != "success" || raw.is_none() {
            if let Ok(mut s) = self.ports.lock() {
                s.cooldown
                    .insert(port.to_owned(), Instant::now() + Duration::from_secs(15));
            }
            return Err(PanelError::new("AT_FAILED", 503));
        }
        let raw = raw.ok_or_else(unavailable)?;
        unchanged()?;
        if raw.split(['\r', '\n']).any(|line| {
            line == "ERROR" || line.starts_with("+CME ERROR:") || line.starts_with("+CMS ERROR:")
        }) {
            return Err(PanelError::new("AT_UNSUPPORTED", 422));
        }
        self.notices.observe(row, command, raw);
        Ok(raw.to_owned())
    }
}

pub fn bounded_read(path: &Path, limit: usize) -> Result<String> {
    let file = fs::File::open(path).map_err(|_| unavailable())?;
    let mut raw = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(|_| unavailable())?;
    if raw.len() > limit {
        return Err(unavailable());
    }
    String::from_utf8(raw).map_err(|_| unavailable())
}
