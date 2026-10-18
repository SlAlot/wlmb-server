//! 落叶小通魔改 modem 数据源；只消费参数缓存，邻区借用原事务工具，任何控制请求均拒绝。
use crate::{PanelError, Result, catalog, luoye_info, radio, sampler::Source};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(unix)]
mod native;
#[cfg(unix)]
pub use native::SystemIo;

pub struct Input {
    pub raw: String,
    pub published_at_ms: Option<u64>,
    pub instance: String,
    pub identity_valid: bool,
}
pub struct ToolReply {
    pub success: bool,
    pub value: Value,
}
pub trait Io: Send + Sync {
    fn load(&self) -> Result<Input>;
    fn neighbor(&self, request: &Value) -> Result<ToolReply>;
    fn neighbors_enabled(&self) -> bool {
        true
    }
}
fn error(code: &'static str) -> PanelError {
    PanelError::new(code, 503)
}
const QUERIES: &[&str] = &[
    "AT+QENG=\"neighbourcell\"",
    "AT^MONNC",
    "AT+GTCCINFO?",
    "AT!NRINFO?",
    "AT!LTEINFO?",
    "AT+CCED=0,2",
];
fn profile(row: &Value) -> Option<&str> {
    let family = row["profile"]["family"].as_str()?;
    let manufacturer = row["manufacturer"].as_str()?;
    if manufacturer != family && !(family == "tdtech" && manufacturer == "huawei") {
        return None;
    }
    [
        "quectel", "fibocom", "tdtech", "huawei", "sierra", "openluat",
    ]
    .contains(&manufacturer)
    .then_some(manufacturer)
}
fn commands(row: &Value) -> Vec<String> {
    if profile(row).is_none()
        || catalog::radio_capabilities(row["model"].as_str().unwrap_or(""))["neighbors"] != true
    {
        return vec![];
    }
    row["profile"]["queries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|cmd| QUERIES.contains(cmd))
        .map(str::to_owned)
        .collect()
}
pub fn request_valid(request: &Value) -> bool {
    request.as_object().is_some_and(|v| v.len() == 5)
        && request["port"] == "/dev/modem-at"
        && request["timeout"] == 4
        && request["baud"] == 115200
        && request["profile"].as_str().is_some_and(|v| {
            [
                "quectel", "fibocom", "tdtech", "huawei", "sierra", "openluat",
            ]
            .contains(&v)
        })
        && request["commands"]
            .as_array()
            .is_some_and(|v| v.len() == 1 && v[0].as_str().is_some_and(|v| QUERIES.contains(&v)))
}
pub fn response(reply: &ToolReply, command: &str) -> Result<String> {
    let message = reply.value["error"].as_str().unwrap_or("");
    if !reply.success
        || !message.is_empty()
        || !reply.value["error"].is_null() && !reply.value["error"].is_string()
    {
        return Err(error(match message {
            "modem transaction queue busy"
            | "modem initialization not ready"
            | "modem transaction busy; retry after initialization" => "AT_BUSY",
            "modem rejected AT command" => "AT_UNSUPPORTED",
            _ => "MODEM_NEIGHBOR_RESULT_UNKNOWN",
        }));
    }
    let raw = reply.value["result"]
        .as_str()
        .filter(|v| v.len() <= crate::MAX_AT_RESPONSE && !v.contains('\0'))
        .ok_or_else(|| error("AT_RESPONSE_MISMATCH"))?;
    let rows: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect();
    if rows.last() != Some(&"OK")
        || rows.iter().filter(|v| **v == "OK").count() != 1
        || rows.iter().any(|v| {
            ["ERROR", "BUSY", "NO CARRIER", "NO DIALTONE"].contains(v)
                || v.starts_with("+CME ERROR:")
                || v.starts_with("+CMS ERROR:")
        })
        || rows
            .iter()
            .any(|v| !crate::sampling::reply_matches_query(command, v))
        || !crate::sampling::reply_matches_query(command, raw)
        || command == "AT+QENG=\"neighbourcell\""
            && rows
                .iter()
                .any(|v| v.starts_with("+QENG:") && !v.contains("\"neighbourcell"))
    {
        return Err(error("AT_RESPONSE_MISMATCH"));
    }
    Ok(raw.to_owned())
}
struct Inventory {
    binding: Value,
    identity: Vec<String>,
    rows: Value,
    at: u64,
}
#[derive(Default)]
struct State {
    row: Option<Value>,
    inventory: Option<Inventory>,
    next: Option<Instant>,
    uncertain: bool,
    failure: Option<&'static str>,
}
pub struct Luoye {
    io: Arc<dyn Io>,
    state: Mutex<State>,
    gate: Mutex<()>,
}
impl Luoye {
    pub fn new(io: Arc<dyn Io>) -> Arc<Self> {
        Arc::new(Self {
            io,
            state: Mutex::new(State::default()),
            gate: Mutex::new(()),
        })
    }
    fn binding(row: &Value) -> Value {
        json!([
            row["model"],
            row["manufacturer"],
            row["instance"],
            row["infoNeighborQuery"]
        ])
    }
    fn read(&self) -> Result<(Value, Value, Input)> {
        let input = self.io.load()?;
        let payload = luoye_info::payload(&input.raw)?;
        let base = &payload["base_info"];
        if luoye_info::text(&base["at_port"]) != Some("/dev/modem-at") {
            return Err(error("MODEM_CACHE_IDENTITY_INVALID"));
        }
        let model = luoye_info::text(&base["name"])
            .filter(|v| !v.is_empty() && v.len() <= 128)
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        let manufacturer = luoye_info::text(&base["manufacturer"])
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut row = json!({"id":"luoye-modem","port":"/dev/modem-at","model":model,"manufacturer":manufacturer,
            "instance":input.instance,"profile":catalog::identify(&model),"shared":false,"channelReady":false,
            "identityVerified":false,"controlReadOnly":true,"smsReadSupported":false,
            "infoTransport":luoye_info::PROVIDER,"discoveredBy":luoye_info::PROVIDER,"infoReadOnly":true,"infoCarrierQuery":true});
        row["infoNeighborQuery"] = json!(
            input.identity_valid && self.io.neighbors_enabled() && !commands(&row).is_empty()
        );
        let mut state = self.state.lock().map_err(|_| error("SYSTEM_BUSY"))?;
        if state
            .row
            .as_ref()
            .is_some_and(|old| Self::binding(old) != Self::binding(&row))
        {
            state.inventory = None;
            // 型号或能力字段短暂缺失不代表原事务已结束，不能借此解除未知结果的暂停。
            if !state.uncertain
                || state
                    .row
                    .as_ref()
                    .is_some_and(|old| old["instance"] != row["instance"])
            {
                state.next = None;
                state.uncertain = false;
                state.failure = None;
            }
        }
        state.row = Some(row.clone());
        Ok((row, payload, input))
    }
    fn add_inventory(&self, row: &Value, value: &mut Value) -> Result<()> {
        let state = self.state.lock().map_err(|_| error("SYSTEM_BUSY"))?;
        if let Some(errors) = value["errors"].as_array_mut() {
            errors.retain(|e| e["source"] != "luoye-modem-transaction");
        }
        let supported = row["infoNeighborQuery"] == true;
        value["neighborState"] = json!({"source":"luoye-modem-transaction","status":if supported {"pending"}else{"unsupported"}});
        if supported {
            value["neighborState"]["queryStatus"] = json!("pending");
        }
        if !supported {
            return Ok(());
        }
        if let Some(old) = state.inventory.as_ref().filter(|old| {
            old.binding == Self::binding(row)
                && Some(&old.identity) == luoye_info::identity(value).as_ref()
        }) {
            value["neighbors"] = old.rows.clone();
            value["neighborState"] = json!({"source":"luoye-modem-transaction","status":if old.rows.as_array().is_some_and(Vec::is_empty){"empty"}else{"reported"},
                "observedAtMs":old.at,"stale":crate::events::wall_ms().saturating_sub(old.at)>15000});
        }
        if let Some(code) = state.failure {
            if value["neighbors"].as_array().is_none_or(Vec::is_empty) {
                value["neighborState"]["status"] = json!("unavailable");
            }
            value["neighborState"]["queryStatus"] = json!("error");
            value["neighborState"]["code"] = json!(code);
            value["neighborState"]["automaticRetryPaused"] = json!(state.uncertain);
            value["partial"] = json!(true);
            value["errors"]
                .as_array_mut()
                .ok_or_else(|| error("MODEM_CACHE_INVALID"))?
                .push(json!({"code":code,"source":"luoye-modem-transaction"}));
        }
        value["partial"] = json!(
            value["serving"].as_array().is_none_or(Vec::is_empty)
                || value["errors"]
                    .as_array()
                    .is_some_and(|errors| !errors.is_empty())
        );
        Ok(())
    }
}
impl Source for Luoye {
    fn modems(&self) -> Result<Vec<Value>> {
        match self.read() {
            Ok((row,_,_))=>Ok(vec![row]),
            Err(_)=>Ok(vec![self.state.lock().map_err(|_|error("SYSTEM_BUSY"))?.row.clone().unwrap_or_else(||json!({
                "id":"luoye-modem","port":"/dev/modem-at","model":"unknown","instance":"unavailable",
                "shared":false,"channelReady":false,"identityVerified":false,"controlReadOnly":true,"smsReadSupported":false,
                "infoTransport":luoye_info::PROVIDER,"infoReadOnly":true,"infoNeighborQuery":false,"infoCarrierQuery":true}))]),
        }
    }
    fn query(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String> {
        Err(PanelError::new("FEATURE_NOT_READY", 422))
    }
    fn info_snapshot(&self, row: &Value) -> Result<Option<Value>> {
        self.info_snapshot_progress(row, &mut |_| Ok(()))
    }
    fn info_snapshot_progress(
        &self,
        expected: &Value,
        publish: &mut dyn FnMut(Value) -> Result<()>,
    ) -> Result<Option<Value>> {
        let (row, payload, input) = self.read()?;
        if Self::binding(expected) != Self::binding(&row) {
            return Err(PanelError::new("MODEM_CHANGED", 409));
        }
        let mut value = luoye_info::parse(
            &payload,
            input.published_at_ms,
            input.identity_valid,
            crate::events::wall_ms(),
        )?;
        value["modem"] = row.clone();
        self.add_inventory(&row, &mut value)?;
        let due = {
            let s = self.state.lock().map_err(|_| error("SYSTEM_BUSY"))?;
            !s.uncertain && s.next.is_none_or(|at| at <= Instant::now())
        };
        if value["stale"] == true || row["infoNeighborQuery"] != true || !due {
            return Ok(Some(value));
        }
        let Some(identity) = luoye_info::identity(&value) else {
            return Ok(Some(value));
        };
        publish(value.clone())?;
        let Ok(_guard) = self.gate.try_lock() else {
            return Ok(Some(value));
        };
        {
            let mut state = self.state.lock().map_err(|_| error("SYSTEM_BUSY"))?;
            if state.uncertain || state.next.is_some_and(|at| at > Instant::now()) {
                return Ok(Some(value));
            }
            state.next = Some(Instant::now() + Duration::from_secs(10));
        }
        let mut neighbors = vec![];
        let mut awaiting = false;
        let read = (|| -> Result<()> {
            for command in commands(&row) {
                let (current, cache, input) = self.read()?;
                let current_value = luoye_info::parse(
                    &cache,
                    input.published_at_ms,
                    input.identity_valid,
                    crate::events::wall_ms(),
                )?;
                if current_value["stale"] == true
                    || Self::binding(&current) != Self::binding(&row)
                    || luoye_info::identity(&current_value) != Some(identity.clone())
                {
                    return Err(error("MODEM_CHANGED"));
                }
                let request = json!({"port":"/dev/modem-at","profile":profile(&row),"commands":[command],"timeout":4,"baud":115200});
                if !request_valid(&request) {
                    return Err(error("AT_WRITE_DISABLED"));
                }
                let at = crate::events::wall_ms();
                awaiting = true;
                let raw = response(&self.io.neighbor(&request)?, &command)?;
                awaiting = false;
                let parsed =
                    radio::parse(row["profile"]["parser"].as_str().unwrap_or("generic"), &raw)?;
                let rows = parsed["neighbors"]
                    .as_array()
                    .filter(|v| v.len() <= 512)
                    .ok_or_else(|| error("AT_PARSE_UNAVAILABLE"))?;
                if rows.is_empty() {
                    return Err(error("MODEM_NEIGHBOR_NOT_REPORTED"));
                }
                for neighbor in rows {
                    let mut neighbor = neighbor.clone();
                    neighbor["observedAtMs"] = json!(at);
                    neighbor["source"] = json!("luoye-modem-transaction");
                    neighbor["stale"] = json!(false);
                    neighbors.push(neighbor);
                }
                if neighbors.len() > 128 {
                    return Err(error("MODEM_NEIGHBOR_TOO_LARGE"));
                }
                let (current, cache, input) = self.read()?;
                let after = luoye_info::parse(
                    &cache,
                    input.published_at_ms,
                    input.identity_valid,
                    crate::events::wall_ms(),
                )?;
                if after["stale"] == true
                    || Self::binding(&current) != Self::binding(&row)
                    || luoye_info::identity(&after) != Some(identity.clone())
                {
                    return Err(error("MODEM_CHANGED"));
                }
            }
            Ok(())
        })();
        let mut state = self.state.lock().map_err(|_| error("SYSTEM_BUSY"))?;
        if state
            .row
            .as_ref()
            .is_none_or(|current| Self::binding(current) != Self::binding(&row))
        {
            if awaiting {
                state.uncertain = true;
                state.failure = Some("MODEM_NEIGHBOR_RESULT_UNKNOWN");
            }
            return Err(error("MODEM_CHANGED"));
        }
        match read {
            Ok(()) => {
                neighbors.retain(|neighbor| {
                    !value["serving"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|serving| {
                            serving["rat"] == neighbor["rat"]
                                && serving["channel"] == neighbor["channel"]
                                && serving["pci"] == neighbor["pci"]
                        })
                });
                let mut seen = std::collections::BTreeSet::new();
                neighbors
                    .retain(|r| seen.insert(json!([r["rat"], r["channel"], r["pci"]]).to_string()));
                let at = neighbors
                    .iter()
                    .filter_map(|r| r["observedAtMs"].as_u64())
                    .min()
                    .unwrap_or_else(crate::events::wall_ms);
                state.inventory = Some(Inventory {
                    binding: Self::binding(&row),
                    identity,
                    rows: json!(neighbors),
                    at,
                });
                state.failure = None;
            }
            Err(e) => {
                state.failure = Some(e.code);
                state.uncertain = awaiting
                    && !["AT_BUSY", "AT_UNSUPPORTED", "AT_WRITE_DISABLED"].contains(&e.code);
                state.next = Some(
                    Instant::now()
                        + Duration::from_secs(if e.code == "AT_BUSY" { 30 } else { 300 }),
                );
                if e.code == "MODEM_CHANGED" {
                    state.inventory = None;
                    return Err(e);
                }
            }
        }
        drop(state);
        self.add_inventory(&row, &mut value)?;
        Ok(Some(value))
    }
}
