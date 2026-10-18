//! 原生服务入口与缓存接口；展示读取和设备写操作保持隔离。
use crate::{
    API_PORT, PROTOCOL, PanelError, Result, VERSION,
    boundary::Lan,
    events::wall_ms,
    http::{self, Handler},
    sampler::Sampler,
    security::{Security, Ubus},
    telemetry,
    transport::{Broker, Bus, NativeBus, SystemDiscovery, bounded_read},
};
use serde_json::{Value, json};
use std::{
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
fn failed() -> PanelError {
    PanelError::new("SYSTEM_UNAVAILABLE", 503)
}
struct AuthBus(Arc<dyn Bus>);
impl Ubus for AuthBus {
    fn call(&mut self, object: &str, method: &str, params: Value) -> Result<Value> {
        self.0.call(object, method, params, Duration::from_secs(5))
    }
}
pub struct NativeService {
    pub sampler: Arc<Sampler>,
    bus: Arc<dyn Bus>,
    sessions: Arc<Mutex<Security>>,
    system: Mutex<Value>,
    born: Instant,
    sms: Option<Arc<crate::sms_runtime::SmsHub>>,
    forwarding: Option<Arc<crate::sms_runtime::ForwardRuntime>>,
    actions: crate::actions::Actions,
    controls: Option<Arc<crate::control_runtime::ModemControls>>,
    host: Option<Arc<crate::host_controls::HostControls>>,
    settings: Arc<Mutex<Value>>,
    settings_io: Option<Arc<dyn crate::host_controls::HostIo>>,
    connection: Mutex<Value>,
}
impl NativeService {
    pub fn local(&self, method: &str, body: Value) -> Result<Value> {
        use sha2::{Digest, Sha256};
        let owner = body["ubus_rpc_session"]
            .as_str()
            .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(|s| format!("{:x}", Sha256::digest(format!("rpc:{s}"))));
        self.authorized(method, body, owner.as_deref())
    }
    fn authorized(&self, method: &str, body: Value, owner: Option<&str>) -> Result<Value> {
        match method {
            "ping" => Ok(json!({"version":VERSION,"backend":"rust"})),
            "status" => {
                let modem = body
                    .get("modem")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty());
                let mut status = self.sampler.status(modem)?;
                status["protocol"] = json!(PROTOCOL);
                status["version"] = json!(VERSION);
                status["backend"] = json!("rust");
                status["connection"] = self.connection.lock().map_err(|_| failed())?.clone();
                let mut system = self.system.lock().map_err(|_| failed())?.clone();
                let at = wall_ms();
                system["stale"] = json!(
                    system["observedAtMs"]
                        .as_u64()
                        .is_none_or(|old| at < old || at - old > 15000)
                );
                if system["lan"].is_object() {
                    crate::lan::age_snapshot(&mut system["lan"], at);
                }
                status["system"] = system;
                if let Some(host) = &self.host {
                    let info = host.public()?;
                    status["system"]["traffic"] = info["traffic"].clone();
                    status["system"]["trafficLimit"] = info["trafficLimit"].clone();
                    if !status["system"]["lan"].is_object() {
                        status["system"]["lan"] = json!({"stale":true});
                    }
                    status["system"]["lan"]["controls"] = info["wifiControls"].clone();
                    status["system"]["lan"]["blocked"] = info["blocked"].clone();
                    status["system"]["lan"]["blockApplyError"] = info["blockApplyError"].clone();
                    status["system"]["lan"]["deviceBlockReason"] =
                        info["deviceBlockReason"].clone();
                    let mut wireless = status["system"]["lan"]["wireless"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    for profile in info["wifiControls"]["displayProfiles"]
                        .as_array()
                        .into_iter()
                        .flatten()
                    {
                        if let Some(row) = wireless.iter_mut().find(|r| r["id"] == profile["id"]) {
                            for field in ["ssid", "band"] {
                                if row[field].is_null() || row[field] == "" {
                                    row[field] = profile[field].clone();
                                }
                            }
                            row["encryption"] = profile["authMode"].clone();
                            row["authMode"] = profile["authMode"].clone();
                            row["configuredEnabled"] = profile["enabled"].clone();
                            row["configuredChannelWidth"] = profile["channelWidth"].clone();
                        } else {
                            let mut row = profile.clone();
                            row["source"] = json!("uci-configuration");
                            row["configuredEnabled"] = row["enabled"].clone();
                            row["configuredChannelWidth"] = row["channelWidth"].clone();
                            row["enabled"] = Value::Null;
                            row["stateKnown"] = json!(false);
                            row["encryption"] = profile["authMode"].clone();
                            wireless.push(row);
                        }
                    }
                    status["system"]["lan"]["wireless"] = json!(wireless);
                    if info["wifiControls"]["displayProfiles"]
                        .as_array()
                        .is_some_and(|p| !p.is_empty())
                    {
                        status["system"]["lan"]["wifiReadSupported"] = json!(true);
                    }
                    status["system"]["lan"]["deviceBlockSupported"] =
                        info["deviceBlockSupported"].clone();
                }
                let rows = status["modems"].as_array().cloned().unwrap_or_default();
                let key = status["selectedModem"].as_str().unwrap_or("").to_owned();
                let at_ready = self.controls.is_some()
                    && rows.iter().any(|r| {
                        r["id"] == key && crate::control_runtime::ModemControls::available(r)
                    });
                let lock = self.controls.is_some()
                    && rows.iter().any(|r| {
                        r["id"] == key
                            && crate::radio_controls::RadioControls::supported(r)
                            && crate::lock_protocol::cell_supported(
                                r["model"].as_str().unwrap_or(""),
                            )
                    });
                let band_lock = self.controls.is_some()
                    && rows.iter().any(|r| {
                        r["id"] == key
                            && crate::radio_controls::RadioControls::supported(r)
                            && crate::lock_protocol::band_supported(
                                r["model"].as_str().unwrap_or(""),
                            )
                    });
                if let Some(c) = &self.controls {
                    if let Some(value) = c.radio.state(&key, &rows) {
                        status["radio"]["controls"] = value;
                    }
                }
                if let Some(row) = rows.iter().find(|r| r["id"] == key) {
                    status["radio"]["lockPresets"] = status["radio"]["controls"]
                        .get("presets")
                        .cloned()
                        .unwrap_or_else(|| {
                            crate::controls::presets(row["model"].as_str().unwrap_or(""))
                        });
                }
                let sms_ready = self.sms.is_some()
                    && rows.iter().any(|r| {
                        r["id"] == key
                            && r["smsReadSupported"] != false
                            && crate::control_runtime::ModemControls::available(r)
                    });
                let sms_write = sms_ready
                    && self.sms.as_ref().is_some_and(|s| {
                        rows.iter()
                            .find(|r| r["id"] == key)
                            .is_some_and(|row| s.device.writable_snapshot(row))
                    });
                let radio_scope = rows
                    .iter()
                    .find(|r| r["id"] == key)
                    .map(|r| {
                        if r["infoTransport"] == "qmodem-rpc" {
                            json!({"signal":true,"carrierAggregation":true,"neighbors":false})
                        } else if crate::control_runtime::ModemControls::available(r) {
                            crate::catalog::radio_capabilities(r["model"].as_str().unwrap_or(""))
                        } else {
                            json!({})
                        }
                    })
                    .unwrap_or_else(|| json!({}));
                status["capabilities"] = json!({"signal":radio_scope["signal"]==true,"neighbors":radio_scope["neighbors"]==true,"carrierAggregation":radio_scope["carrierAggregation"]==true,"wifi":status["system"]["lan"]["wifiReadSupported"]==true,"deviceList":status["system"]["lan"]["deviceListSupported"]==true,"deviceBlock":status["system"]["lan"]["deviceBlockSupported"]==true,"sms":sms_ready,"smsSend":sms_write,"smsDelete":sms_write,"smsForward":sms_ready && self.forwarding.is_some() && cfg!(feature="pushplus-https"),"trafficLimit":self.host.is_some(),"lockBand":band_lock,"bandLock":band_lock,"lockCell":lock,"cellLock":lock,"at":at_ready,"configure":self.controls.is_some() && self.settings_io.is_some()});
                status["smsMode"] = json!(if !sms_ready {
                    "unavailable"
                } else if sms_write {
                    if rows.iter().any(|r| r["id"] == key && r["shared"] == true) {
                        "shared-serialized"
                    } else {
                        "dedicated"
                    }
                } else {
                    "compat-read-only"
                });
                status["capabilities"]["smsReadState"] = json!(sms_ready);
                if let Some(sms) = &self.sms {
                    status["smsBackend"] = sms.device.source_info(&key);
                }
                if let Some(runtime) = &self.forwarding {
                    if let Ok(settings) = runtime.public() {
                        let selected = settings["modem"] == key;
                        status["smsCollection"] = json!({"enabled":selected&&(settings["enabled"]==true||settings["receiveEnabled"]==true),
                            "running":selected&&settings["runtime"]["receiveRunning"]==true,
                            "eventConnected":selected&&settings["runtime"]["eventConnected"]==true,
                            "fallbackSeconds":settings["pollSeconds"],"strategy":"event-first"});
                    }
                }
                status["capabilities"]["smsReceivingConfigure"] = json!(
                    self.controls.is_some()
                        && rows.iter().any(|r| r["id"] == key
                            && crate::control_runtime::ModemControls::available(r)
                            && r["controlReadOnly"] != true
                            && crate::sms_settings::supported(r["model"].as_str().unwrap_or("")))
                );
                Ok(status)
            }
            "settings" => {
                let mut v = self.settings.lock().map_err(|_| failed())?.clone();
                v["configureSupported"] =
                    json!(self.controls.is_some() && self.settings_io.is_some());
                v["connection"] = self.connection.lock().map_err(|_| failed())?.clone();
                Ok(v)
            }
            "configure" => {
                let c = self
                    .controls
                    .clone()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?;
                let io = self
                    .settings_io
                    .clone()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?;
                let interval = body["interval"]
                    .as_u64()
                    .filter(|n| (2..=30).contains(n))
                    .ok_or_else(|| PanelError::new("INVALID_REQUEST", 400))?;
                let allow = body["allowUnsafeAt"]
                    .as_bool()
                    .ok_or_else(|| PanelError::new("INVALID_REQUEST", 400))?;
                let settings = self.settings.clone();
                let sampler = self.sampler.clone();
                self.actions.submit(
                    owner.ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?,
                    move || {
                        let value = json!({"version":1,"interval":interval,"allowUnsafeAt":allow});
                        io.save(
                            "settings.json",
                            &serde_json::to_vec(&value).map_err(|_| failed())?,
                        )?;
                        sampler.set_interval(interval)?;
                        c.unsafe_at.store(allow, Ordering::Release);
                        *settings.lock().map_err(|_| failed())? = value;
                        Ok(json!({"saved":true}))
                    },
                )
            }
            "refresh" => {
                let sampler = self.sampler.clone();
                self.actions.submit(
                    owner.ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?,
                    move || {
                        sampler.tick()?;
                        Ok(json!({"queued":true}))
                    },
                )
            }
            "at_prepare" | "at_execute" => {
                let c = self
                    .controls
                    .clone()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?;
                let owner = owner
                    .ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?
                    .to_owned();
                let key = body["modem"].as_str().unwrap_or("").to_owned();
                let command = body["command"].as_str().unwrap_or("").to_owned();
                if method == "at_prepare" {
                    return c.prepare(&key, &command, &owner);
                }
                let nonce = body["confirmation"].as_str().unwrap_or("").to_owned();
                let job_owner = owner.clone();
                self.actions.submit(&job_owner, move || {
                    c.execute(&key, &command, &nonce, &owner)
                })
            }
            "lock_status" | "lock_prepare" | "lock_execute" => {
                let radio = self
                    .controls
                    .as_ref()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?
                    .radio
                    .clone();
                let owner = owner
                    .ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?
                    .to_owned();
                let key = body["modem"].as_str().unwrap_or("").to_owned();
                let operation = if let Some(v) = body["operation"].as_str() {
                    crate::boundary::decode_json(v.as_bytes())?
                } else {
                    body["operation"].clone()
                };
                let mode = method.to_owned();
                let job_owner = owner.clone();
                self.actions
                    .submit(&job_owner, move || match mode.as_str() {
                        "lock_status" => radio.read(&key, body["force"] == true),
                        "lock_prepare" => radio.prepare(&key, &operation, &owner),
                        _ => radio.execute(
                            &key,
                            &operation,
                            body["revision"].as_str().unwrap_or(""),
                            body["confirmation"].as_str().unwrap_or(""),
                            &owner,
                        ),
                    })
            }
            "host_prepare" | "host_execute" => {
                let host = self
                    .host
                    .clone()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?;
                let owner = owner
                    .ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?
                    .to_owned();
                let job_owner = owner.clone();
                let op = if let Some(v) = body["operation"].as_str() {
                    crate::boundary::decode_json(v.as_bytes())?
                } else {
                    body["operation"].clone()
                };
                let prepare = method == "host_prepare";
                self.actions.submit(&job_owner, move || {
                    if prepare {
                        host.prepare(&op, &owner)
                    } else {
                        host.execute(
                            &op,
                            body["revision"].as_str().unwrap_or(""),
                            body["confirmation"].as_str().unwrap_or(""),
                            &owner,
                        )
                    }
                })
            }
            "diagnostics" => Ok(
                json!({"service":"wlmb-server","version":VERSION,"backend":"rust","recentErrors":self.sampler.events.recent()}),
            ),
            "control_result" => self.actions.result(
                owner.ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?,
                body["job"].as_str().unwrap_or(""),
            ),
            "sms_settings_prepare" | "sms_settings_execute" => {
                let controls = self
                    .controls
                    .clone()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?;
                let owner = owner
                    .ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?
                    .to_owned();
                let key = body["modem"].as_str().unwrap_or("").to_owned();
                let enabled = body["enabled"]
                    .as_bool()
                    .ok_or_else(|| PanelError::new("INVALID_REQUEST", 400))?;
                let nonce = body["confirmation"].as_str().unwrap_or("").to_owned();
                let prepare = method == "sms_settings_prepare";
                self.actions.submit(&owner.clone(), move || {
                    if prepare {
                        controls.sms_prepare(&key, enabled, &owner)
                    } else {
                        controls.sms_execute(&key, enabled, &nonce, &owner)
                    }
                })
            }
            "sms_list" => self
                .sms
                .as_ref()
                .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?
                .cached(body["modem"].as_str().unwrap_or("")),
            "sms_refresh" => {
                let sms = self
                    .sms
                    .clone()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?;
                let modem = body["modem"].as_str().unwrap_or("").to_owned();
                self.actions.submit(
                    owner.ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?,
                    move || {
                        let count = sms.refresh(&modem)?.len();
                        Ok(json!({"refreshed":true,"count":count}))
                    },
                )
            }
            "sms_read_state" => {
                let sms = self
                    .sms
                    .as_ref()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?;
                let unread = body["unread"]
                    .as_bool()
                    .ok_or_else(|| PanelError::new("SMS_READ_STATE_INVALID", 422))?;
                sms.mark_read(
                    body["modem"].as_str().unwrap_or(""),
                    body["id"].as_str().unwrap_or(""),
                    unread,
                )
            }
            "sms_prepare" | "sms_execute" => {
                let sms = self
                    .sms
                    .clone()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?;
                if !sms
                    .device
                    .writable_for(body["modem"].as_str().unwrap_or(""))
                {
                    return Err(PanelError::new("SMS_SHARED_READ_ONLY", 422));
                }
                let owner = owner.ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?;
                let modem = body["modem"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 128)
                    .ok_or_else(|| PanelError::new("SMS_SEND_INVALID", 422))?
                    .to_owned();
                let operation = if let Some(raw) = body["operation"].as_str() {
                    crate::boundary::decode_json(raw.as_bytes())?
                } else {
                    body["operation"].clone()
                };
                let number = operation["number"].as_str().unwrap_or("").to_owned();
                let content = operation["content"].as_str().unwrap_or("").to_owned();
                let deleting = operation["type"] == "delete";
                let fingerprint = operation["id"].as_str().unwrap_or("").to_owned();
                if deleting {
                    if fingerprint.len() != 64
                        || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(PanelError::new("SMS_DELETE_INVALID", 422));
                    }
                } else if operation["type"] == "send" {
                    crate::sms_pdu::encode_submit(&number, &content)?;
                } else {
                    return Err(PanelError::new("SMS_OPERATION_UNSUPPORTED", 501));
                }
                use sha2::{Digest, Sha256};
                let revision = sms.device.revision(&modem)?;
                let canonical = serde_json::to_vec(&json!([
                    "sms",
                    deleting,
                    number,
                    content,
                    fingerprint,
                    revision
                ]))
                .map_err(|_| failed())?;
                let binding = format!("sms-send:{:x}", Sha256::digest(canonical));
                let mut sessions = self.sessions.lock().map_err(|_| failed())?;
                if method == "sms_prepare" {
                    let mut value =
                        sessions.prepare(owner, &modem, &binding, self.born.elapsed())?;
                    value["warning"] = json!(if deleting {
                        if sms.device.source_info(&modem)["deleteScope"] == "qmodem-record" {
                            "删除 QModem 短信记录，不保证删除 SIM 上仍存在的副本。不可撤销；超时不会自动重试。"
                        } else {
                            "短信及其完整分段将从设备删除，不可撤销。执行前核对当前短信，超时不会自动重试。"
                        }
                    } else {
                        "发送短信可能产生运营商费用。确认后只发送一次；结果不确定时不会自动重发。"
                    });
                    return Ok(value);
                }
                sessions.consume(
                    body["confirmation"].as_str().unwrap_or(""),
                    owner,
                    &modem,
                    &binding,
                    self.born.elapsed(),
                )?;
                drop(sessions);
                self.actions.submit(owner, move || {
                    if sms.device.revision(&modem)? != revision {
                        return Err(PanelError::new("MODEM_CHANGED", 409));
                    }
                    let v = if deleting {
                        sms.delete(&modem, &fingerprint)?
                    } else {
                        sms.send(&modem, &number, &content)?
                    };
                    Ok(v)
                })
            }
            "sms_forward_settings" => self
                .forwarding
                .as_ref()
                .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?
                .public(),
            "sms_forward_configure" => {
                let runtime = self
                    .forwarding
                    .clone()
                    .ok_or_else(|| PanelError::new("FEATURE_NOT_READY", 501))?;
                self.actions.submit(
                    owner.ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))?,
                    move || runtime.update(&body),
                )
            }
            _ => Err(PanelError::new("FEATURE_NOT_READY", 501)),
        }
    }
    pub fn new(bus: Arc<dyn Bus>, sampler: Arc<Sampler>) -> Arc<Self> {
        Self::with_sms(bus, sampler, None, None)
    }
    pub fn with_sms(
        bus: Arc<dyn Bus>,
        sampler: Arc<Sampler>,
        sms: Option<Arc<crate::sms_runtime::SmsHub>>,
        forwarding: Option<Arc<crate::sms_runtime::ForwardRuntime>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sampler,
            bus,
            sessions: Arc::new(Mutex::new(Security::default())),
            system: Mutex::new(json!({"stale":true})),
            born: Instant::now(),
            sms,
            forwarding,
            actions: crate::actions::Actions::default(),
            controls: None,
            host: None,
            settings: Arc::new(Mutex::new(
                json!({"interval":3,"allowUnsafeAt":false,"development":true}),
            )),
            settings_io: None,
            connection: Mutex::new(json!({"mode":"unknown"})),
        })
    }
    pub fn integrated(
        bus: Arc<dyn Bus>,
        sampler: Arc<Sampler>,
        source: Arc<dyn crate::sampler::Source>,
        sms: Option<Arc<crate::sms_runtime::SmsHub>>,
        forwarding: Option<Arc<crate::sms_runtime::ForwardRuntime>>,
        io: Option<Arc<dyn crate::host_controls::HostIo>>,
    ) -> Result<Arc<Self>> {
        let born = Instant::now();
        let sessions = Arc::new(Mutex::new(Security::default()));
        let controls = crate::control_runtime::ModemControls::new(source, sessions.clone(), born);
        let mut settings = json!({"version":1,"interval":3,"allowUnsafeAt":false});
        if let Some(io) = &io {
            let loaded = (|| -> Result<()> {
                if let Some(raw) = io.load("settings.json")? {
                    let v: Value = serde_json::from_slice(&raw).map_err(|_| failed())?;
                    let interval = v["interval"]
                        .as_u64()
                        .filter(|n| (2..=30).contains(n))
                        .ok_or_else(failed)?;
                    let allow = v["allowUnsafeAt"].as_bool().ok_or_else(failed)?;
                    if v["version"] != 1 {
                        return Err(failed());
                    }
                    sampler.set_interval(interval)?;
                    controls.unsafe_at.store(allow, Ordering::Release);
                    settings = v;
                }
                Ok(())
            })();
            if let Err(e) = loaded {
                sampler.events.record("settings", e.code);
            }
        }
        let host = io.as_ref().and_then(|io| {
            match crate::host_controls::HostControls::new(io.clone(), sessions.clone(), born) {
                Ok(host) => Some(host),
                Err(e) => {
                    sampler.events.record("host-settings", e.code);
                    None
                }
            }
        });
        Ok(Arc::new(Self {
            sampler,
            bus,
            sessions,
            system: Mutex::new(json!({"stale":true})),
            born,
            sms,
            forwarding,
            actions: crate::actions::Actions::default(),
            controls: Some(controls),
            host,
            settings: Arc::new(Mutex::new(settings)),
            settings_io: io,
            connection: Mutex::new(json!({"mode":"unknown"})),
        }))
    }
    fn token<'a>(&self, headers: &'a [(String, String)]) -> Result<&'a str> {
        let values = headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .collect::<Vec<_>>();
        if values.len() != 1 {
            return Err(PanelError::new("SESSION_EXPIRED", 401));
        }
        values[0]
            .1
            .strip_prefix("Bearer ")
            .ok_or_else(|| PanelError::new("SESSION_EXPIRED", 401))
    }
    pub fn sample_host(self: &Arc<Self>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
        let this = self.clone();
        std::thread::spawn(move || {
            let mut old_cpu = None;
            let mut old_net: Option<(String, String, f64, u64, u64)> = None;
            let mut lan_reader = crate::lan::LanCache::default();
            let mut lan_cache = Value::Null;
            let mut lan_at = None;
            let board = this
                .bus
                .call("system", "board", json!({}), Duration::from_secs(3))
                .unwrap_or(Value::Null);
            while !stop.load(Ordering::Relaxed) {
                let cycle_began = Instant::now();
                let mut value = json!({"board":board,"observedAtMs":wall_ms(),"stale":false});
                let cpu = bounded_read(Path::new("/proc/stat"), 32768)
                    .ok()
                    .and_then(|s| telemetry::cpu_times(&s));
                value["cpuUsage"] = json!(cpu.and_then(|c| telemetry::cpu_percent(old_cpu, c)));
                old_cpu = cpu;
                value["memory"] = bounded_read(Path::new("/proc/meminfo"), 16384)
                    .ok()
                    .and_then(|s| telemetry::memory(&s))
                    .unwrap_or(Value::Null);
                value["temperature"] = crate::temperature::device().unwrap_or(Value::Null);
                value["uptimeSeconds"] = json!(
                    bounded_read(Path::new("/proc/uptime"), 256)
                        .ok()
                        .and_then(|s| s
                            .split_whitespace()
                            .next()
                            .and_then(|v| v.parse::<f64>().ok()))
                        .filter(|s| s.is_finite())
                        .map(|s| s as u64)
                );
                let device = this
                    .bus
                    .call(
                        "network.interface",
                        "dump",
                        json!({}),
                        Duration::from_secs(3),
                    )
                    .ok()
                    .and_then(|v| telemetry::wan_device(&v));
                let counters = device.and_then(|device| {
                    let read = |key| {
                        bounded_read(
                            &Path::new("/sys/class/net")
                                .join(&device)
                                .join("statistics")
                                .join(key),
                            128,
                        )
                        .ok()
                        .and_then(|s| s.trim().parse::<u64>().ok())
                    };
                    let index = || {
                        bounded_read(
                            &Path::new("/sys/class/net").join(&device).join("ifindex"),
                            128,
                        )
                        .ok()
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .filter(|i| *i > 0)
                    };
                    let before = index()?;
                    let rx = read("rx_bytes")?;
                    let tx = read("tx_bytes")?;
                    if index()? != before {
                        return None;
                    }
                    Some((
                        device.clone(),
                        format!("{device}#{before}"),
                        this.born.elapsed().as_secs_f64(),
                        rx,
                        tx,
                    ))
                });
                if let Some((device, identity, mono, rx, tx)) = &counters {
                    let previous =
                        old_net
                            .as_ref()
                            .map(|(_, name, at, rx, tx)| telemetry::NetSample {
                                interface: name,
                                mono_seconds: *at,
                                rx: *rx,
                                tx: *tx,
                            });
                    let current = telemetry::NetSample {
                        interface: identity,
                        mono_seconds: *mono,
                        rx: *rx,
                        tx: *tx,
                    };
                    value["network"] = json!({"interface":device,"source":"l3-interface-counter-delta","rates":telemetry::rate(previous.as_ref(),&current),"sinceBootRxBytes":rx,"sinceBootTxBytes":tx});
                } else {
                    value["network"] = Value::Null;
                }
                old_net = counters;
                if let Some(host) = &this.host {
                    let date = crate::transport::process(
                        "/bin/date",
                        &["+%Y-%m-%d"],
                        None,
                        Duration::from_secs(1),
                    )
                    .ok()
                    .and_then(|s| crate::traffic::Date::parse(&s).ok());
                    let counters = old_net
                        .as_ref()
                        .map(|(_, identity, _, rx, tx)| (identity.as_str(), *rx, *tx));
                    if let Err(e) = host.sample(date, counters) {
                        this.sampler.events.record("host", e.code);
                    }
                }
                if lan_at.is_none_or(|at: Duration| {
                    this.born.elapsed().saturating_sub(at) >= Duration::from_secs(12)
                }) {
                    lan_cache =
                        lan_reader.sample(this.bus.as_ref(), this.born.elapsed().as_secs_f64());
                    lan_at = Some(this.born.elapsed());
                }
                value["lan"] = lan_cache.clone();
                if let Ok(mut cache) = this.system.lock() {
                    *cache = value;
                }
                let remaining =
                    telemetry::sample_delay(cycle_began.elapsed(), Duration::from_secs(6));
                let until = Instant::now() + remaining;
                while let Some(left) = until.checked_duration_since(Instant::now()) {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(left.min(Duration::from_millis(100)));
                }
            }
        })
    }
}
impl Handler for NativeService {
    fn handle(
        &self,
        path: &str,
        body: Value,
        headers: &[(String, String)],
        peer: Ipv4Addr,
    ) -> Result<Value> {
        if path == "/health" {
            return Ok(
                json!({"protocol":PROTOCOL,"version":VERSION,"backend":"rust","development":true}),
            );
        }
        if path == "/api/login" {
            let mut bus = AuthBus(self.bus.clone());
            return self.sessions.lock().map_err(|_| failed())?.login(
                &mut bus,
                body["username"].as_str().unwrap_or(""),
                body["password"].as_str().unwrap_or(""),
                peer,
                self.born.elapsed(),
            );
        }
        let token = self.token(headers)?;
        let mut sessions = self.sessions.lock().map_err(|_| failed())?;
        let owner = sessions.authenticate(token, peer, self.born.elapsed())?;
        if path == "/api/logout" {
            sessions.logout(token);
            return Ok(json!({"loggedOut":true}));
        }
        drop(sessions);
        self.authorized(path.strip_prefix("/api/").unwrap_or(""), body, Some(&owner))
    }
}
pub fn lan(bus: &dyn Bus) -> Result<Lan> {
    let status = bus.call(
        "network.interface.lan",
        "status",
        json!({}),
        Duration::from_secs(5),
    )?;
    let rows = status["ipv4-address"].as_array().ok_or_else(failed)?;
    if rows.len() != 1 {
        return Err(PanelError::new("LAN_UNAVAILABLE", 503));
    }
    let address = rows[0]["address"]
        .as_str()
        .ok_or_else(failed)?
        .parse::<Ipv4Addr>()
        .map_err(|_| failed())?;
    let mask = u8::try_from(rows[0]["mask"].as_u64().ok_or_else(failed)?).map_err(|_| failed())?;
    Lan::new(address, mask)
}
pub fn run_readonly() -> Result<()> {
    run(Some(false))
}
pub fn run_standalone() -> Result<()> {
    run(Some(true))
}
pub fn run_auto() -> Result<()> {
    run(None)
}
fn run(requested_standalone: Option<bool>) -> Result<()> {
    let bus = Arc::new(NativeBus::default());
    let qmodem = crate::connection_mode::qmodem_installed(Path::new("/"));
    let glinet_detected = requested_standalone != Some(true)
        && !qmodem
        && crate::glinet::Glinet::detected(bus.as_ref())
            .unwrap_or_else(|_| Path::new("/etc/glversion").is_file());
    let backend = crate::connection_mode::backend(requested_standalone, qmodem, glinet_detected);
    let glinet = (backend == crate::connection_mode::Backend::Glinet)
        .then(|| crate::glinet::Glinet::new(bus.clone()));
    let standalone = backend == crate::connection_mode::Backend::Independent;
    let lan = lan(bus.as_ref())?;
    let listener = TcpListener::bind((lan.address, API_PORT))
        .map_err(|_| PanelError::new("PORT_BUSY", 503))?;
    let independent = standalone.then(|| {
        crate::independent_at::IndependentSource::new(Arc::new(
            crate::independent_at::SystemHardware,
        ))
    });
    let broker = (!standalone && glinet.is_none()).then(|| {
        Arc::new(Broker::new(
            bus.clone(),
            Arc::new(SystemDiscovery {
                proc_root: PathBuf::from("/proc"),
            }),
        ))
    });
    let source: Arc<dyn crate::sampler::Source> = if let Some(source) = &glinet {
        source.clone()
    } else if let Some(source) = &independent {
        source.clone()
    } else {
        broker.clone().ok_or_else(failed)?
    };
    let sampler = Sampler::new(source.clone());
    let sms_device: Option<Arc<dyn crate::sms_runtime::SmsDevice>> = glinet
        .clone()
        .map(|source| source as Arc<dyn crate::sms_runtime::SmsDevice>)
        .or_else(|| {
            independent
                .map(|source| source as Arc<dyn crate::sms_runtime::SmsDevice>)
                .or_else(|| {
                    broker.map(|b| {
                        let fallback = Arc::new(crate::compat_sms::CompatSms::new(b.clone()));
                        Arc::new(crate::qmodem_sms::QmodemSms::new(
                            Arc::new(crate::qmodem_sms::NativeService::new(b)),
                            fallback,
                        )) as Arc<dyn crate::sms_runtime::SmsDevice>
                    })
                })
        });
    let sms = sms_device.and_then(|device| {
        let persistent = crate::private_store::PrivateStore::open(Path::new("/etc/wlmb-server"))
            .and_then(|store| {
                crate::sms_runtime::SmsHub::with_read_store(device.clone(), Arc::new(store))
            });
        match persistent {
            Ok(hub) => Some(hub),
            Err(e) => {
                sampler.events.record("sms-read-state", e.code);
                None
            }
        }
    });
    let forwarding = if let Some(hub) = &sms {
        let dir = Path::new("/etc/wlmb-server");
        let configured = (|| {
            crate::sms_runtime::ForwardRuntime::new(
                hub.clone(),
                Arc::new(crate::sms_runtime::DiskStore::open(dir)?),
                Arc::new(crate::sms_runtime::NativeFactory {
                    directory: dir.into(),
                }),
            )
        })();
        match configured {
            Ok(runtime) => Some(runtime),
            Err(e) => {
                sampler.events.record("sms-forward", e.code);
                None
            }
        }
    } else {
        None
    };
    let io = crate::private_store::PrivateStore::open(Path::new("/etc/wlmb-server"))
        .inspect_err(|e| sampler.events.record("host-settings", e.code))
        .ok()
        .map(|store| {
            crate::native_host::NativeHost::new(bus.clone(), store)
                as Arc<dyn crate::host_controls::HostIo>
        });
    let service = NativeService::integrated(
        bus.clone(),
        sampler.clone(),
        source,
        sms,
        forwarding.clone(),
        io,
    )?;
    let cleanup = service.clone();
    *service.connection.lock().map_err(|_| failed())? = json!({
        "mode":if standalone {"independent"} else {"shared"},
        "qmodemDetected":qmodem,
        "selection":"startup",
        "busyPortPolicy":"never-steal",
        "sharedService":if standalone {Value::Null} else if glinet.is_some() {json!("glinet-modem-at")} else {json!("at-daemon")}
    });
    let stop = Arc::new(AtomicBool::new(false));
    let term = signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone())
        .map_err(|_| failed())?;
    let interrupt = signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())
        .map_err(|_| failed())?;
    let control = crate::rpc::serve(service.clone(), stop.clone())?;
    let sample_worker = {
        let sampler = sampler.clone();
        std::thread::spawn(move || sampler.run())
    };
    let host = service.sample_host(stop.clone());
    if let Some(runtime) = &forwarding {
        if let Err(e) = runtime.start_saved() {
            sampler.events.record("sms-forward", e.code);
        }
    }
    let result = http::serve(listener, lan, service, stop.clone());
    stop.store(true, Ordering::Relaxed);
    let _ = control.join();
    if !cleanup.actions.shutdown(Duration::from_secs(40)) {
        cleanup
            .sampler
            .events
            .record("host", "CONTROL_DRAIN_TIMEOUT");
    }
    if let Some(runtime) = forwarding {
        runtime.shutdown();
    }
    let _ = sampler.stop(Duration::from_secs(8));
    let _ = sample_worker.join();
    let _ = host.join();
    if let Some(host) = &cleanup.host {
        host.shutdown();
    }
    let _ = bus.close();
    signal_hook::low_level::unregister(term);
    signal_hook::low_level::unregister(interrupt);
    result
}
