//! 单一后台采集与快照缓存；按命令有效期采样，客户端不叠加查询。
use crate::{
    PanelError, Result, catalog,
    events::{Events, wall_ms},
    radio,
    sampling::{self, Sample},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

pub trait Source: Send + Sync {
    fn modems(&self) -> Result<Vec<Value>>;
    fn query(&self, id: &str, command: &str, port: &str, model: &str) -> Result<String>;
    fn query_bound(&self, row: &Value, command: &str) -> Result<String> {
        if !self
            .modems()?
            .iter()
            .any(|r| r["id"] == row["id"] && signature(r) == signature(row))
        {
            return Err(PanelError::new("MODEM_CHANGED", 409));
        }
        self.query(
            row["id"].as_str().unwrap_or(""),
            command,
            row["port"].as_str().unwrap_or(""),
            row["model"].as_str().unwrap_or(""),
        )
    }
}
#[cfg(unix)]
impl Source for crate::transport::Broker {
    fn query_bound(&self, row: &Value, command: &str) -> Result<String> {
        self.query_bound(
            row,
            command,
            if command.starts_with("AT+QNWLOCK=") {
                10
            } else {
                3
            },
        )
    }
    fn modems(&self) -> Result<Vec<Value>> {
        self.modems()
    }
    fn query(&self, id: &str, command: &str, port: &str, model: &str) -> Result<String> {
        self.query(id, command, Some((port, model)))
    }
}
#[derive(Clone)]
struct Entry {
    modem: Value,
    generation: u64,
}
#[derive(Clone)]
struct Queries {
    generation: u64,
    samples: BTreeMap<String, Sample>,
    identity: Value,
    last_read_recovery: Option<Instant>,
}
struct Cached {
    generation: u64,
    value: Value,
    finished: Instant,
}
struct State {
    entries: Vec<Entry>,
    sequence: u64,
    cache: HashMap<String, Cached>,
    queries: HashMap<String, Queries>,
    jobs: HashMap<String, u64>,
    unsupported: HashMap<(String, u64, String), Instant>,
    cursor: usize,
    interval: u64,
    stopping: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            entries: vec![],
            sequence: 0,
            cache: HashMap::new(),
            queries: HashMap::new(),
            jobs: HashMap::new(),
            unsupported: HashMap::new(),
            cursor: 0,
            interval: 3,
            stopping: false,
        }
    }
}
pub struct Sampler {
    source: Arc<dyn Source>,
    state: Mutex<State>,
    changed: Condvar,
    pub events: Events,
}
fn failed() -> PanelError {
    PanelError::new("SAMPLE_FAILED", 503)
}
fn id(modem: &Value) -> Result<&str> {
    modem["id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .ok_or_else(failed)
}
fn signature(modem: &Value) -> (&Value, &Value, &Value) {
    (&modem["port"], &modem["model"], &modem["instance"])
}
fn current(state: &State, key: &str, generation: u64) -> bool {
    state
        .entries
        .iter()
        .any(|e| e.modem["id"] == key && e.generation == generation)
}
impl Sampler {
    pub fn new(source: Arc<dyn Source>) -> Arc<Self> {
        Arc::new(Self {
            source,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            events: Events::default(),
        })
    }
    pub fn set_interval(&self, seconds: u64) -> Result<()> {
        if !(2..=30).contains(&seconds) {
            return Err(PanelError::new("INVALID_REQUEST", 400));
        }
        self.state.lock().map_err(|_| failed())?.interval = seconds;
        self.changed.notify_all();
        Ok(())
    }
    pub fn discover(&self) -> Result<()> {
        let mut rows = self.source.modems()?;
        if rows.len() > 64 {
            return Err(failed());
        }
        let mut seen = std::collections::HashSet::new();
        for modem in &mut rows {
            if !seen.insert(id(modem)?.to_owned()) {
                return Err(failed());
            }
            let model = modem["model"]
                .as_str()
                .filter(|s| s.len() <= 128)
                .ok_or_else(failed)?;
            modem["profile"] = catalog::identify(model);
        }
        let mut state = self.state.lock().map_err(|_| failed())?;
        if state.stopping {
            return Ok(());
        }
        let mut entries = vec![];
        for modem in rows {
            let old = state
                .entries
                .iter()
                .find(|e| e.modem["id"] == modem["id"] && signature(&e.modem) == signature(&modem));
            let generation = if let Some(old) = old {
                old.generation
            } else {
                state.sequence = state.sequence.checked_add(1).ok_or_else(failed)?;
                state.sequence
            };
            entries.push(Entry { modem, generation });
        }
        let active = entries
            .iter()
            .filter_map(|e| id(&e.modem).ok().map(|key| (key.to_owned(), e.generation)))
            .collect::<HashMap<_, _>>();
        state
            .cache
            .retain(|key, value| active.get(key) == Some(&value.generation));
        state
            .queries
            .retain(|key, value| active.get(key) == Some(&value.generation));
        state.unsupported.retain(|(key, generation, _), until| {
            active.get(key) == Some(generation) && *until > Instant::now()
        });
        state.entries = entries;
        Ok(())
    }
    pub fn refresh(self: &Arc<Self>, key: &str) -> Result<Value> {
        let entry = self
            .state
            .lock()
            .map_err(|_| failed())?
            .entries
            .iter()
            .find(|e| e.modem["id"] == key)
            .cloned()
            .ok_or_else(|| PanelError::new("MODEM_NOT_FOUND", 404))?;
        self.queue(entry)
    }
    fn queue(self: &Arc<Self>, entry: Entry) -> Result<Value> {
        let key = id(&entry.modem)?.to_owned();
        {
            let mut state = self.state.lock().map_err(|_| failed())?;
            if state.stopping || !current(&state, &key, entry.generation) {
                return Ok(json!({"accepted":false,"busy":false}));
            }
            if state.jobs.contains_key(&key) || state.jobs.len() >= 4 {
                return Ok(json!({"accepted":false,"busy":true}));
            }
            if state
                .cache
                .get(&key)
                .is_some_and(|c| c.finished.elapsed() < Duration::from_millis(800))
            {
                return Ok(json!({"accepted":false,"busy":false}));
            }
            state.jobs.insert(key.clone(), entry.generation);
        }
        let sampler = self.clone();
        let job_key = key.clone();
        let generation = entry.generation;
        if std::thread::Builder::new()
            .name("wlmb-sample".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    sampler.collect(&entry)
                }));
                if !matches!(result, Ok(Ok(()))) {
                    sampler.failed_sample(&entry);
                }
                if let Ok(mut state) = sampler.state.lock() {
                    if state.jobs.get(&job_key) == Some(&generation) {
                        state.jobs.remove(&job_key);
                    }
                    sampler.changed.notify_all();
                }
            })
            .is_err()
        {
            if let Ok(mut state) = self.state.lock() {
                state.jobs.remove(&key);
            }
            return Err(failed());
        }
        Ok(json!({"accepted":true,"busy":true}))
    }
    fn failed_sample(&self, entry: &Entry) {
        self.events.record("sampler", "SAMPLE_FAILED");
        if let Ok(mut state) = self.state.lock() {
            let Ok(key) = id(&entry.modem) else {
                return;
            };
            if state.stopping || !current(&state, key, entry.generation) {
                return;
            }
            let mut value = state.cache.get(key).map(|v| v.value.clone()).unwrap_or_else(|| json!({"modem":entry.modem,"serving":[],"carriers":[],"neighbors":[],"observations":[],"observedAtMs":wall_ms()}));
            value["errors"] = json!([{"code":"SAMPLE_FAILED"}]);
            value["stale"] = json!(true);
            value["partial"] = json!(true);
            value["pending"] = json!(false);
            state.cache.insert(
                key.to_owned(),
                Cached {
                    generation: entry.generation,
                    value,
                    finished: Instant::now(),
                },
            );
        }
    }
    fn collect(&self, entry: &Entry) -> Result<()> {
        let key = id(&entry.modem)?;
        let parser = entry.modem["profile"]["parser"]
            .as_str()
            .unwrap_or("generic");
        let commands = if entry.modem["profile"]["parser"].is_null() {
            vec!["AT+CSQ".to_owned(), "AT+COPS?".to_owned()]
        } else {
            entry.modem["profile"]["queries"]
                .as_array()
                .ok_or_else(failed)?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        };
        if commands.is_empty() || commands.len() > crate::MAX_SAMPLE_QUERIES {
            return Err(failed());
        }
        let mut previous = self
            .state
            .lock()
            .map_err(|_| failed())?
            .queries
            .get(key)
            .filter(|q| q.generation == entry.generation)
            .cloned()
            .unwrap_or(Queries {
                generation: entry.generation,
                samples: BTreeMap::new(),
                identity: Value::Null,
                last_read_recovery: None,
            });
        let mut errors = vec![];
        for (index, command) in commands.iter().enumerate() {
            let (interval, disabled) = {
                let state = self.state.lock().map_err(|_| failed())?;
                if state.stopping || !current(&state, key, entry.generation) {
                    return Ok(());
                }
                (
                    state.interval,
                    state
                        .unsupported
                        .get(&(key.to_owned(), entry.generation, command.clone()))
                        .is_some_and(|until| *until > Instant::now()),
                )
            };
            let ttl = Duration::from_secs(catalog::query_ttl(command, interval, index == 0));
            if disabled {
                if index == 0 && matches!(parser, "sierra-em91" | "sierra-em92") {
                    previous.samples.remove(command);
                }
                errors.push(json!({"command":command,"code":"AT_UNSUPPORTED"}));
                continue;
            }
            if previous
                .samples
                .get(command)
                .is_some_and(|s| s.mono.elapsed() < ttl)
            {
                continue;
            }
            let model = entry.modem["model"].as_str().ok_or_else(failed)?;
            // 已有有效主温度回复时不再发送备用命令；格式不兼容才继续，忙碌或超时仍停止本轮。
            let temperature_queries = crate::temperature::queries(model);
            if let Some(position) = temperature_queries
                .iter()
                .position(|query| *query == command)
            {
                if temperature_queries[..position].iter().any(|query| {
                    previous.samples.get(*query).is_some_and(|sample| {
                        sample.mono.elapsed() < Duration::from_secs(60)
                            && crate::temperature::response(model, query, &sample.raw).is_some()
                    })
                }) {
                    continue;
                }
            }
            let query = || {
                let raw = self.source.query_bound(&entry.modem, command)?;
                if !sampling::reply_matches_query(command, &raw) {
                    return Err(PanelError::new("AT_RESPONSE_MISMATCH", 503));
                }
                Ok(raw)
            };
            let mut reply = query();
            // 只读主测量遇到无效回复时至多补查一次，每模组三十秒最多一次；写操作和超时不重放。
            if index == 0
                && parser != "generic"
                && !matches!(parser, "sierra-em91" | "sierra-em92")
                && crate::security::read_only(command)
                && previous
                    .last_read_recovery
                    .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
                && (reply
                    .as_ref()
                    .is_err_and(|e| e.code == "AT_RESPONSE_MISMATCH")
                    || reply.as_ref().ok().is_some_and(|raw| {
                        radio::parse(parser, raw).ok().is_some_and(|value| {
                            value["serving"].as_array().is_some_and(Vec::is_empty)
                        })
                    }))
            {
                previous.last_read_recovery = Some(Instant::now());
                self.events.record("sampler", "AT_READ_RECOVERY");
                std::thread::sleep(Duration::from_millis(150));
                {
                    let state = self.state.lock().map_err(|_| failed())?;
                    if state.stopping || !current(&state, key, entry.generation) {
                        return Ok(());
                    }
                }
                reply = query();
            }
            match reply {
                Ok(raw) => {
                    let alternate = matches!(parser, "sierra-em91" | "sierra-em92");
                    if index == 0
                        || alternate && index == 1 && !previous.samples.contains_key(&commands[0])
                    {
                        let primary = radio::parse(parser, &raw)?;
                        if parser != "generic"
                            && primary["serving"]
                                .as_array()
                                .is_none_or(|rows| rows.is_empty())
                        {
                            self.events.record("sampler", "AT_PARSE_UNAVAILABLE");
                            // 仅记录回复结构，便于区分空回复与格式差异，不记录原文或用户数据。
                            let response_kind = if raw.trim().is_empty() {
                                "empty"
                            } else if raw
                                .split(['\r', '\n'])
                                .filter(|line| !line.trim().is_empty())
                                .all(|line| line.trim() == "OK" || line.trim() == command)
                            {
                                "terminal-only"
                            } else {
                                "unrecognized"
                            };
                            errors.push(json!({"command":command,"code":"AT_PARSE_UNAVAILABLE","responseKind":response_kind,"responseBytes":raw.len()}));
                            previous.samples.remove(command);
                            if alternate && index == 0 {
                                continue;
                            }
                            break;
                        }
                        let identity = json!(
                            primary["serving"]
                                .as_array()
                                .ok_or_else(failed)?
                                .iter()
                                .map(|r| json!([r["rat"], r["channel"], r["pci"]]))
                                .collect::<Vec<_>>()
                        );
                        if identity != previous.identity {
                            previous.samples.clear();
                        }
                        previous.identity = identity;
                    }
                    previous.samples.insert(
                        command.clone(),
                        Sample {
                            raw,
                            wall_ms: wall_ms(),
                            mono: Instant::now(),
                        },
                    );
                    self.publish(entry, &commands, &previous.samples, &errors)?;
                }
                Err(e) => {
                    self.events.record("sampler", e.code);
                    errors.push(json!({"command":command,"code":e.code}));
                    if e.code == "AT_UNSUPPORTED" {
                        if index == 0 && matches!(parser, "sierra-em91" | "sierra-em92") {
                            previous.samples.remove(command);
                        }
                        let mut state = self.state.lock().map_err(|_| failed())?;
                        if current(&state, key, entry.generation) {
                            state.unsupported.insert(
                                (key.to_owned(), entry.generation, command.clone()),
                                Instant::now() + Duration::from_secs(300),
                            );
                        }
                    } else {
                        break;
                    }
                }
            }
        }
        previous.samples.retain(|command, s| {
            s.mono.elapsed()
                < Duration::from_secs(if crate::temperature::slow_query(command) {
                    90
                } else {
                    30
                })
        });
        self.publish(entry, &commands, &previous.samples, &errors)?;
        let mut state = self.state.lock().map_err(|_| failed())?;
        if !state.stopping && current(&state, key, entry.generation) {
            state.queries.insert(key.to_owned(), previous);
        }
        Ok(())
    }
    fn publish(
        &self,
        entry: &Entry,
        commands: &[String],
        samples: &BTreeMap<String, Sample>,
        errors: &[Value],
    ) -> Result<()> {
        let key = id(&entry.modem)?;
        let mut value = sampling::publish(&entry.modem, commands, samples, errors, wall_ms())?;
        let mut state = self.state.lock().map_err(|_| failed())?;
        if state.stopping || !current(&state, key, entry.generation) {
            return Ok(());
        }
        if sampling::primary_query(&entry.modem, commands, samples, wall_ms())
            .and_then(|command| samples.get(command))
            .is_none_or(|s| s.mono.elapsed() >= Duration::from_secs(30))
        {
            if let Some(old) = state.cache.get(key).filter(|v| {
                v.value["observations"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
            }) {
                value = old.value.clone();
                value["errors"] = json!(errors);
                value["stale"] = json!(true);
                value["partial"] = json!(true);
            }
        }
        state.cache.insert(
            key.to_owned(),
            Cached {
                generation: entry.generation,
                value,
                finished: Instant::now(),
            },
        );
        Ok(())
    }
    pub fn status(&self, requested: Option<&str>) -> Result<Value> {
        let state = self.state.lock().map_err(|_| failed())?;
        let entry = match requested {
            Some(key) => Some(
                state
                    .entries
                    .iter()
                    .find(|e| e.modem["id"] == key)
                    .ok_or_else(|| PanelError::new("MODEM_NOT_READY", 409))?,
            ),
            None => state.entries.first(),
        };
        let key = entry.and_then(|e| id(&e.modem).ok());
        let busy = key.is_some_and(|k| state.jobs.contains_key(k));
        let mut value = key.and_then(|k|state.cache.get(k)).filter(|c|entry.is_some_and(|e|e.generation==c.generation))
            .map(|c|c.value.clone()).unwrap_or_else(||json!({"serving":[],"carriers":[],"neighbors":[],"errors":[],"pending":key.is_some(),"stale":true,"ageMs":null}));
        let now = wall_ms();
        if let Some(at) = value["observedAtMs"].as_u64() {
            let age = now.saturating_sub(at);
            value["ageMs"] = json!(age);
            value["stale"] = json!(value["stale"] == true || age > 15000 || now < at);
            value["pending"] = json!(busy);
            for group in ["serving", "carriers", "neighbors"] {
                if let Some(rows) = value[group].as_array_mut() {
                    for row in rows {
                        let age = row["observedAtMs"]
                            .as_u64()
                            .filter(|at| *at <= now)
                            .map(|at| now - at);
                        row["ageMs"] = json!(age);
                        row["stale"] = json!(row["stale"] == true || age.is_none_or(|a| a > 15000));
                        sampling::age_metrics(row, now);
                    }
                }
            }
            if let Some(row) = value
                .get_mut("secondaryMeasurement")
                .filter(|row| row.is_object())
            {
                row["stale"] = json!(
                    row["stale"] == true
                        || row["observedAtMs"]
                            .as_u64()
                            .is_none_or(|at| now < at || now - at > 15000)
                );
            }
            for field in [
                "subscription",
                "sessionQos",
                "smsReceiving",
                "networkOperator",
            ] {
                if let Some(subscription) = value.get_mut(field) {
                    let ttl = if field == "networkOperator" && subscription["source"] != "AT+COPS?"
                    {
                        15000
                    } else {
                        90000
                    };
                    subscription["stale"] = json!(
                        subscription["stale"] == true
                            || subscription["observedAtMs"]
                                .as_u64()
                                .is_none_or(|at| now < at || now - at > ttl)
                    );
                }
            }
        }
        sampling::age_temperature(&mut value, now);
        if value["networkOperator"]["stale"] == true {
            value["operator"] = Value::Null;
        }
        Ok(
            json!({"modems":state.entries.iter().map(|e|e.modem.clone()).collect::<Vec<_>>(),"selectedModem":key,"radio":value}),
        )
    }
    pub fn tick(self: &Arc<Self>) -> Result<()> {
        self.discover()?;
        let rows = {
            let mut state = self.state.lock().map_err(|_| failed())?;
            let count = state.entries.len();
            let start = if count == 0 { 0 } else { state.cursor % count };
            let rows = state.entries[start..]
                .iter()
                .chain(&state.entries[..start])
                .filter(|e| {
                    e.modem["shared"] == true
                        || e.modem["transport"] == "dedicated"
                            && e.modem["channelReady"] == true
                            && e.modem["identityVerified"] == true
                })
                .cloned()
                .collect::<Vec<_>>();
            state.cursor = start + 4;
            rows
        };
        for entry in rows {
            self.queue(entry)?;
        }
        Ok(())
    }
    pub fn run(self: &Arc<Self>) -> Result<()> {
        loop {
            if self.state.lock().map_err(|_| failed())?.stopping {
                return Ok(());
            }
            if let Err(e) = self.tick() {
                self.events.record("discovery", e.code);
            }
            let state = self.state.lock().map_err(|_| failed())?;
            if state.stopping {
                return Ok(());
            }
            let delay = Duration::from_secs(state.interval);
            let interval = state.interval;
            let _ = self
                .changed
                .wait_timeout_while(state, delay, |s| !s.stopping && s.interval == interval)
                .map_err(|_| failed())?;
        }
    }
    pub fn stop(&self, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().map_err(|_| failed())?;
        state.stopping = true;
        self.changed.notify_all();
        while !state.jobs.is_empty() {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(false);
            };
            let result = self
                .changed
                .wait_timeout(state, left)
                .map_err(|_| failed())?;
            state = result.0;
        }
        Ok(true)
    }
}
