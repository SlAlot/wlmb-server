//! 参考QModem公开平台协议的锁定调度；选择、确认、回读与实际射频生效分开。
use crate::{PanelError, Result, controls, events::wall_ms, sampler::Source, security::Security};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
struct Cached {
    signature: (String, String, Value),
    observed: Instant,
    value: Value,
}
pub struct RadioControls {
    source: Arc<dyn Source>,
    security: Arc<Mutex<Security>>,
    enabled: Arc<AtomicBool>,
    started: Instant,
    gate: Mutex<()>,
    cache: Mutex<HashMap<String, Cached>>,
}
fn error(code: &'static str, status: u16) -> PanelError {
    PanelError::new(code, status)
}
fn signature(row: &Value) -> Result<(String, String, Value)> {
    let port = row["port"]
        .as_str()
        .ok_or_else(|| error("LOCK_UNSUPPORTED", 422))?;
    let model = row["model"]
        .as_str()
        .ok_or_else(|| error("LOCK_UNSUPPORTED", 422))?;
    Ok((port.to_owned(), model.to_owned(), row["instance"].clone()))
}
impl RadioControls {
    pub fn new(
        source: Arc<dyn Source>,
        security: Arc<Mutex<Security>>,
        enabled: Arc<AtomicBool>,
        started: Instant,
    ) -> Self {
        Self {
            source,
            security,
            enabled,
            started,
            gate: Mutex::new(()),
            cache: Mutex::new(HashMap::new()),
        }
    }
    fn modem(&self, key: &str) -> Result<Value> {
        let rows = self.source.modems()?;
        if rows.len() > 64 {
            return Err(error("LOCK_UNSUPPORTED", 422));
        }
        self.prune(&rows);
        let row = rows
            .into_iter()
            .find(|row| row["id"] == key)
            .ok_or_else(|| error("LOCK_UNSUPPORTED", 422))?;
        let model = row["model"]
            .as_str()
            .ok_or_else(|| error("LOCK_UNSUPPORTED", 422))?;
        if !Self::supported(&row)
            || !(crate::lock_protocol::cell_supported(model)
                || crate::lock_protocol::band_supported(model))
        {
            return Err(error("LOCK_UNSUPPORTED", 422));
        }
        Ok(row)
    }
    pub fn supported(row: &Value) -> bool {
        row["controlReadOnly"] != true
            && row["channelState"] != "sms-input-unknown"
            && (crate::lock_protocol::cell_supported(row["model"].as_str().unwrap_or(""))
                || crate::lock_protocol::band_supported(row["model"].as_str().unwrap_or("")))
            && (row["shared"] == true
                || row["transport"] == "dedicated"
                    && row["channelReady"] == true
                    && row["identityVerified"] == true)
    }
    pub fn prune(&self, rows: &[Value]) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.retain(|key, v| {
                v.observed.elapsed() < Duration::from_secs(60)
                    && rows.iter().any(|row| {
                        row["id"] == key.as_str()
                            && signature(row).ok().as_ref() == Some(&v.signature)
                    })
            });
        }
    }
    pub fn state(&self, key: &str, rows: &[Value]) -> Option<Value> {
        self.prune(rows);
        self.cache.lock().ok()?.get(key).map(|v| v.value.clone())
    }
    pub fn read(&self, key: &str, force: bool) -> Result<Value> {
        let row = self.modem(key)?;
        if !force {
            let sig = signature(&row)?;
            if let Some(value) = self
                .cache
                .lock()
                .map_err(|_| error("LOCK_BUSY", 409))?
                .get(key)
                .filter(|v| v.signature == sig && v.observed.elapsed() < Duration::from_secs(30))
                .map(|v| v.value.clone())
            {
                return Ok(value);
            }
        }
        let _held = self.gate.try_lock().map_err(|_| error("LOCK_BUSY", 409))?;
        self.read_locked(&row)
    }
    fn read_locked(&self, row: &Value) -> Result<Value> {
        let key = row["id"]
            .as_str()
            .ok_or_else(|| error("LOCK_UNSUPPORTED", 422))?;
        let sig = signature(row)?;
        let mut groups = serde_json::Map::new();
        let mut errors = vec![];
        let mut cell_replies = HashMap::<String, Result<String>>::new();
        let model = row["model"].as_str().unwrap_or("");
        let presets = controls::presets(model);
        for rat in ["NR", "LTE"] {
            if !crate::lock_protocol::cell_supported(model) {
                continue;
            }
            if presets[rat].as_array().is_none_or(|bands| bands.is_empty())
                && !(crate::vendor_cell_lock::supported(model)
                    && (rat == "LTE" || crate::catalog::platform(model) != "lte"))
            {
                continue;
            }
            let command = crate::lock_protocol::query(model, rat);
            if command.is_empty() {
                continue;
            }
            match cell_replies
                .entry(command.clone())
                .or_insert_with(|| self.source.query_bound(row, &command))
                .clone()
                .and_then(|raw| crate::lock_protocol::parse(model, &raw, rat))
            {
                Ok(value) => {
                    groups.insert(rat.into(), value);
                }
                Err(e) => {
                    if [
                        "AT_BUSY",
                        "SYSTEM_BUSY",
                        "AT_COOLDOWN",
                        "AT_NOT_READY",
                        "MODEM_CHANGED",
                        "MODEM_NOT_FOUND",
                    ]
                    .contains(&e.code)
                    {
                        return Err(e);
                    }
                    errors.push(json!({"rat":rat,"code":e.code}));
                }
            }
        }
        let mut band_replies = std::collections::BTreeMap::new();
        let mut band_state = json!({"groups":{},"wire":{}});
        if crate::band_locks::supported(model) {
            for command in crate::band_locks::queries(model) {
                match self.source.query_bound(row, &command) {
                    Ok(raw) => {
                        band_replies.insert(command, raw);
                    }
                    Err(e)
                        if [
                            "AT_BUSY",
                            "SYSTEM_BUSY",
                            "AT_COOLDOWN",
                            "AT_NOT_READY",
                            "MODEM_CHANGED",
                            "MODEM_NOT_FOUND",
                        ]
                        .contains(&e.code) =>
                    {
                        return Err(e);
                    }
                    Err(e) => errors.push(json!({"target":"band","code":e.code})),
                }
            }
            match crate::band_locks::parse(model, &band_replies) {
                Ok(value) => band_state = value,
                Err(e) => errors.push(json!({"target":"band","code":e.code})),
            }
        } else if crate::catalog::normalize(model) == "mt5700m-cn" {
            band_state["groups"] = json!(groups);
        }
        if !self
            .source
            .modems()?
            .iter()
            .any(|r| r["id"] == key && signature(r).ok().as_ref() == Some(&sig))
        {
            return Err(error("MODEM_CHANGED", 409));
        }
        let value = json!({"groups":groups,"errors":errors,"observedAtMs":wall_ms(),"activation":"unverified","requiresExternalActivation":!crate::lock_protocol::immediate(model),"scsRequired":crate::lock_protocol::scs_required(model),"maxCells":crate::lock_protocol::max_cells(model),"persistent":!crate::lock_protocol::immediate(model),"note":if crate::lock_protocol::immediate(model){"此协议锁定可能立即影响供网且不持久保存，不设置开机重放，不自动重连；仅回读已确认的协议字段。"}else{"保存配置不代表当前射频已生效；本插件不会自动切飞行模式或重启。"}});
        let mut value = value;
        value["cellFields"] = crate::lock_protocol::cell_fields(model);
        value["bandGroups"] = band_state["groups"].clone();
        value["bandWire"] = band_state["wire"].clone();
        value["bandAutoScope"] = json!(if crate::band_locks::modem_reset(model) {
            "modem"
        } else {
            "rat"
        });
        value["cellWriteScope"] = json!(if crate::vendor_cell_lock::global_write(model, "cell") {
            "modem"
        } else {
            "rat"
        });
        value["bandCellExclusive"] = json!(crate::catalog::normalize(model) == "mt5700m-cn");
        let mut presets = presets;
        if let Some(groups) = band_state["groups"].as_object() {
            for (rat, group) in groups {
                let all = group["availableBands"].as_array().filter(|a| !a.is_empty());
                if let Some(all) = all {
                    presets[match rat.as_str() {
                        "NR_NSA" => "NRNSA",
                        _ => rat,
                    }] = json!(all);
                    if rat == "NR" {
                        presets["NRSA"] = json!(all);
                    }
                    if group["availableSource"] == "firmware" {
                        presets["source"] = json!("negotiated-firmware");
                    }
                }
            }
            if groups.contains_key("NR_NSA") {
                let mut nr = presets["NRSA"].as_array().cloned().unwrap_or_default();
                for band in presets["NRNSA"].as_array().into_iter().flatten() {
                    if !nr.contains(band) {
                        nr.push(band.clone());
                    }
                }
                presets["NR"] = json!(nr);
            }
        }
        crate::band_inventory::cell_metadata(&mut presets, model);
        value["presets"] = presets;
        self.cache
            .lock()
            .map_err(|_| error("LOCK_BUSY", 409))?
            .insert(
                key.into(),
                Cached {
                    signature: sig,
                    observed: Instant::now(),
                    value: value.clone(),
                },
            );
        Ok(value)
    }
    fn writable(&self) -> Result<()> {
        if self.enabled.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(error("AT_WRITE_DISABLED", 403))
        }
    }
    fn revision(row: &Value, state: &Value, operation: &Value) -> Result<String> {
        let rat = operation["rat"]
            .as_str()
            .ok_or_else(|| error("LOCK_INVALID", 400))?;
        let band = crate::band_locks::operation(operation)
            && crate::band_locks::supported(row["model"].as_str().unwrap_or(""));
        let model = row["model"].as_str().unwrap_or("");
        let global = if band {
            operation["mode"] == "auto" && crate::band_locks::modem_reset(model)
        } else {
            crate::vendor_cell_lock::global_write(model, operation["mode"].as_str().unwrap_or(""))
        };
        let group_key = if band { "bandGroups" } else { "groups" };
        let bytes=serde_json::to_vec(&json!({"port":row["port"],"model":row["model"],"instance":row["instance"],"group":if global {state[group_key].clone()}else{state[group_key][rat].clone()},"wire":if band {state["bandWire"].clone()}else{Value::Null},"operation":operation})).map_err(|_|error("LOCK_INVALID",400))?;
        Ok(Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    }
    pub fn prepare(&self, key: &str, operation: &Value, owner: &str) -> Result<Value> {
        self.writable()?;
        let row = self.modem(key)?;
        controls::validate_preset(row["model"].as_str().unwrap_or(""), operation)?;
        if !(crate::band_locks::operation(operation)
            && crate::band_locks::supported(row["model"].as_str().unwrap_or("")))
        {
            crate::lock_protocol::command(row["model"].as_str().unwrap_or(""), operation)?;
        }
        let state = self.read(key, true)?;
        Self::validate_live(&row, operation, &state)?;
        let band = crate::band_locks::operation(operation)
            && crate::band_locks::supported(row["model"].as_str().unwrap_or(""));
        let commands = Self::commands(&row, operation, &state)?;
        if state[if band { "bandGroups" } else { "groups" }]
            .get(
                operation["rat"]
                    .as_str()
                    .ok_or_else(|| error("LOCK_INVALID", 400))?,
            )
            .is_none()
        {
            return Err(error("LOCK_UNSUPPORTED", 422));
        }
        let revision = Self::revision(&row, &state, operation)?;
        self.writable()?;
        let mut result = self
            .security
            .lock()
            .map_err(|_| error("LOCK_BUSY", 409))?
            .prepare(
                owner,
                key,
                &format!("LOCK:{revision}"),
                self.started.elapsed(),
            )?;
        result["revision"] = json!(revision);
        result["operation"] = operation.clone();
        result["command"] = json!(commands.join("\n"));
        result["warning"] = json!(format!(
            "将覆盖此制式锁定配置，可能影响供网。{}",
            state["note"].as_str().unwrap_or("不会自动重置网络。")
        ));
        if band {
            result["warning"] = json!(
                "将修改所选制式的频段限制，可能暂时断开连接；不会自动拨号、切飞行或重启。恢复自动频段不解除小区锁定。"
            );
        }
        if crate::catalog::normalize(row["model"].as_str().unwrap_or("")) == "mt5700m-cn"
            && operation["mode"] == "band"
            && state["groups"][operation["rat"].as_str().unwrap_or("")]["mode"] == 2
        {
            result["warning"] = json!(
                "此固件的频段与小区锁共用配置，保存频段将替换当前制式的小区锁，可能影响供网；不会自动拨号、切飞行或重启。请确认后保存。"
            );
        }
        if band
            && commands
                .iter()
                .any(|c| c == "AT+ZBAND=0" || c.starts_with("AT+ZBAND=all,"))
        {
            result["warning"] = json!(
                "此固件的命令会同时恢复其他制式的频段选择，可能暂时影响连接；不会解除小区锁定，不自动拨号或重启。请确认后保存。"
            );
        }
        if band && commands.iter().any(|command| command == "AT!BAND=0F") {
            result["warning"] = json!(
                "将保存并选择自定义频段模板，可能立即影响供网；同时保留读取到的其他制式掩码，不自动飞行、拨号或重启。部分失败不会自动重放，请刷新状态确认。"
            );
        }
        if operation["mode"] == "auto"
            && crate::vendor_cell_lock::modem_reset(row["model"].as_str().unwrap_or(""))
            && !band
        {
            result["warning"] = json!(
                "此模组使用全局小区解锁指令，将解除 4G 和 5G 小区锁定；不修改频段限制，不自动拨号或重启。"
            );
        } else if !band
            && crate::vendor_cell_lock::global_write(row["model"].as_str().unwrap_or(""), "cell")
        {
            result["warning"] = json!(
                "此模组只保存一个全局小区目标，新目标会替换原有 4G / 5G 小区锁定，可能立即影响供网；不修改频段限制，不自动拨号或重启。"
            );
        }
        Ok(result)
    }
    pub fn execute(
        &self,
        key: &str,
        operation: &Value,
        revision: &str,
        confirmation: &str,
        owner: &str,
    ) -> Result<Value> {
        self.writable()?;
        let row = self.modem(key)?;
        controls::validate_preset(row["model"].as_str().unwrap_or(""), operation)?;
        if revision.len() != 64
            || !revision
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        {
            return Err(error("LOCK_CONFIRMATION_REQUIRED", 409));
        }
        self.security
            .lock()
            .map_err(|_| error("LOCK_BUSY", 409))?
            .consume(
                confirmation,
                owner,
                key,
                &format!("LOCK:{revision}"),
                self.started.elapsed(),
            )?;
        let _held = self.gate.try_lock().map_err(|_| error("LOCK_BUSY", 409))?;
        let state = self.read_locked(&row)?;
        if revision != Self::revision(&row, &state, operation)? {
            return Err(error("LOCK_CHANGED", 409));
        }
        Self::validate_live(&row, operation, &state)?;
        let commands = Self::commands(&row, operation, &state)?;
        self.writable()?;
        self.cache
            .lock()
            .map_err(|_| error("LOCK_BUSY", 409))?
            .remove(key);
        for (index, command) in commands.iter().enumerate() {
            let raw = self.source.query_bound(&row, command).map_err(|e| {
                if index > 0 {
                    return error("LOCK_PARTIAL", 409);
                }
                if e.code == "AT_UNSUPPORTED" {
                    error("LOCK_REJECTED", 422)
                } else {
                    error("LOCK_RESULT_UNKNOWN", 409)
                }
            })?;
            if !controls::write_accepted(&raw, command) {
                return Err(error("LOCK_RESULT_UNKNOWN", 409));
            }
        }
        let matched = self.read_locked(&row).ok().is_some_and(|after| {
            if crate::band_locks::supported(row["model"].as_str().unwrap_or(""))
                && crate::band_locks::operation(operation)
            {
                return crate::band_locks::matched_transition(
                    row["model"].as_str().unwrap_or(""),
                    &json!({"groups":state["bandGroups"],"wire":state["bandWire"]}),
                    &json!({"groups":after["bandGroups"],"wire":after["bandWire"]}),
                    operation,
                ) && state["groups"].as_object().is_some_and(|groups| {
                    groups.iter().all(|(rat, before)| {
                        before["mode"] == after["groups"][rat]["mode"]
                            && before["entries"] == after["groups"][rat]["entries"]
                    })
                });
            }
            if operation["mode"] == "auto"
                && crate::vendor_cell_lock::modem_reset(row["model"].as_str().unwrap_or(""))
            {
                return state["groups"].as_object().is_some_and(|groups| {
                    !groups.is_empty()
                        && groups.keys().all(|rat| {
                            after["groups"][rat]["mode"] == 0
                                && after["groups"][rat]["entries"]
                                    .as_array()
                                    .is_some_and(Vec::is_empty)
                        })
                });
            }
            let group = &after["groups"][operation["rat"].as_str().unwrap_or("")];
            if crate::lock_protocol::immediate(row["model"].as_str().unwrap_or("")) {
                return crate::lock_protocol::matched(
                    row["model"].as_str().unwrap_or(""),
                    group,
                    operation,
                );
            }
            let expected = match operation["mode"].as_str() {
                Some("auto") => 0,
                Some("band") => 3,
                _ => 2,
            };
            let same_entries = group["entries"]
                .as_array()
                .zip(operation["entries"].as_array())
                .is_some_and(|(actual, requested)| {
                    actual.len() == requested.len()
                        && requested.iter().all(|entry| actual.contains(entry))
                });
            group["mode"] == expected
                && same_entries
                && (expected == 0 || group["forbidFlag"] == if expected == 3 { 1 } else { 0 })
        });
        let mut result = json!({"accepted":true,"savedVerified":matched,"persistent":!crate::lock_protocol::immediate(row["model"].as_str().unwrap_or("")),"activation":"unverified","requiresExternalActivation":!crate::lock_protocol::immediate(row["model"].as_str().unwrap_or("")),"note":if matched {"配置已接受且可报告字段回读一致；射频生效未做硬件验收，未重置网络。"} else {"配置已接受，回读未确认；请先刷新状态，不要直接重复提交。"}});
        if crate::band_locks::operation(operation)
            && crate::band_locks::supported(row["model"].as_str().unwrap_or(""))
        {
            result["persistent"] = Value::Null;
            result["requiresExternalActivation"] = Value::Null;
        }
        result["acceptedAtMs"] = json!(wall_ms());
        Ok(result)
    }
    fn commands(row: &Value, operation: &Value, state: &Value) -> Result<Vec<String>> {
        let model = row["model"]
            .as_str()
            .ok_or_else(|| error("LOCK_UNSUPPORTED", 422))?;
        if crate::band_locks::supported(model) && crate::band_locks::operation(operation) {
            crate::band_locks::commands(
                model,
                operation,
                &json!({"groups":state["bandGroups"],"wire":state["bandWire"]}),
            )
        } else {
            Ok(vec![crate::lock_protocol::command(model, operation)?])
        }
    }
    fn validate_live(row: &Value, op: &Value, state: &Value) -> Result<()> {
        if crate::catalog::normalize(row["model"].as_str().unwrap_or("")) == "mt5700m-cn"
            && op["target"] == "band"
            && op["mode"] == "auto"
            && [1, 2]
                .iter()
                .any(|mode| state["groups"][op["rat"].as_str().unwrap_or("")]["mode"] == *mode)
        {
            return Err(error("LOCK_BAND_CELL_CONFLICT", 422));
        }
        if op["mode"] != "cell" {
            return Ok(());
        }
        let model = row["model"].as_str().unwrap_or("");
        let rat = op["rat"].as_str().unwrap_or("");
        if !crate::vendor_cell_lock::supported(model) {
            return Ok(());
        }
        let profile = crate::controls::presets(model);
        let allowed = if state["bandGroups"][rat]["availableSource"] == "firmware" {
            &state["bandGroups"][rat]["availableBands"]
        } else {
            &profile[rat]
        };
        let allowed = allowed
            .as_array()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| error("LOCK_BAND_INVENTORY_REQUIRED", 422))?;
        if op["entries"].as_array().is_none_or(|rows| {
            rows.iter().any(|r| {
                if rat == "LTE" && r["band"].is_null() {
                    return !r["arfcn"].as_u64().is_some_and(|ch| {
                        crate::radio::band_candidates(rat, ch as f64)
                            .iter()
                            .any(|b| {
                                b[1..]
                                    .parse::<u64>()
                                    .is_ok_and(|band| allowed.contains(&json!(band)))
                            })
                    });
                }
                !allowed.contains(&r["band"])
            })
        }) {
            return Err(error("LOCK_BAND_UNSUPPORTED", 422));
        }
        Ok(())
    }
}
