//! 插件自有WiFi与上网控制；不接管拨号，不修改其他插件规则。
use crate::{
    PanelError, Result,
    security::Security,
    traffic::{Accounting, Budget, Date},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Instant,
};
fn bad() -> PanelError {
    PanelError::new("HOST_INVALID", 422)
}
fn changed() -> PanelError {
    PanelError::new("HOST_CHANGED", 409)
}
pub fn identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub fn interface(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
pub fn mac(s: &str) -> bool {
    let parts: Vec<_> = s.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
        && u8::from_str_radix(parts[0], 16).is_ok_and(|v| v & 1 == 0)
        && s != "00:00:00:00:00:00"
}
fn exact(v: &Value, keys: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|m| m.len() == keys.len() && keys.iter().all(|k| m.contains_key(*k)))
}
pub fn wifi_disabled(value: &Value) -> bool {
    value == true || value == 1 || value == "1" || value == "true"
}
pub fn profiles(config: &Value) -> Vec<Value> {
    let Some(rows) = config["values"].as_object() else {
        return vec![];
    };
    if rows.len() > 64 {
        return vec![];
    }
    let mut result = vec![];
    for (id, row) in rows {
        let radio = row["device"].as_str().unwrap_or("");
        let interfaces = rows
            .values()
            .filter(|r| {
                r[".type"] == "wifi-iface"
                    && (r["device"] == radio
                        || r["disabled"] != "1"
                            && r["device"]
                                .as_array()
                                .is_some_and(|a| a.iter().any(|v| v == radio)))
            })
            .count();
        let device = &config["values"][radio];
        let band = match device["band"].as_str().or(device["hwmode"].as_str()) {
            Some("2g" | "11g") => "2g",
            Some("5g" | "11a") => "5g",
            _ => continue,
        };
        let net = &row["network"];
        let lan = net == "lan"
            || net
                .as_array()
                .is_some_and(|a| a.len() == 1 && a[0] == "lan");
        if row[".type"] != "wifi-iface"
            || !identifier(id)
            || !identifier(radio)
            || interfaces != 1
            || !lan
            || row["mode"] != "ap"
            || device[".type"] != "wifi-device"
            || !["psk2", "psk2+ccmp", "sae", "sae-mixed"]
                .contains(&row["encryption"].as_str().unwrap_or(""))
            || ["mld", "mld_id", "mlo", "mesh_id", "auth_server", "wds"]
                .iter()
                .any(|k| row.get(k).is_some_and(|v| v != "0" && v != ""))
        {
            continue;
        }
        result.push(json!({"id":id,"radio":radio,"band":band,"ssid":row["ssid"],"enabled":!wifi_disabled(&row["disabled"]) && !wifi_disabled(&device["disabled"]),"channel":device["channel"],"channelWidth":device["htmode"],"authMode":row["encryption"]}));
    }
    let counts = result
        .iter()
        .fold(BTreeMap::<String, usize>::new(), |mut m, r| {
            *m.entry(r["band"].as_str().unwrap_or("").into())
                .or_default() += 1;
            m
        });
    result.retain(|r| counts.get(r["band"].as_str().unwrap_or("")) == Some(&1));
    result
}

pub fn wifi_unified(snapshot: &Value) -> Option<bool> {
    let p = profiles(&snapshot["wireless"]);
    let a = p.iter().find(|r| r["band"] == "2g")?;
    let b = p.iter().find(|r| r["band"] == "5g")?;
    let row = |r: &Value| &snapshot["wireless"]["values"][r["id"].as_str().unwrap_or("")];
    let (a_row, b_row) = (row(a), row(b));
    let (a_key, b_key) = (a_row["key"].as_str()?, b_row["key"].as_str()?);
    Some(
        a["enabled"] == true
            && b["enabled"] == true
            && a["ssid"].as_str().is_some_and(|s| !s.is_empty())
            && a["ssid"] == b["ssid"]
            && a["authMode"] == b["authMode"]
            && !a_key.is_empty()
            && a_key == b_key,
    )
}
fn separated_ssid(value: &str) -> String {
    let mut prefix = String::new();
    for c in value.chars() {
        if prefix.len() + c.len_utf8() > 29 {
            break;
        }
        prefix.push(c)
    }
    format!("{prefix}_5G")
}
pub fn wifi_changes(snapshot: &Value, op: &Value) -> Result<Value> {
    if !exact(op, &["kind", "band", "changes"]) || op["kind"] != "wifi_edit" {
        return Err(bad());
    }
    let profile = editable_profiles(snapshot)
        .into_iter()
        .find(|r| r["band"] == op["band"])
        .ok_or_else(|| PanelError::new("WIFI_EDIT_UNSUPPORTED", 422))?;
    let changes = op["changes"]
        .as_object()
        .filter(|m| !m.is_empty() && m.len() <= 7)
        .ok_or_else(bad)?;
    let id = profile["id"].as_str().ok_or_else(bad)?;
    let encryption = changes
        .get("authMode")
        .unwrap_or(&profile["authMode"])
        .as_str()
        .ok_or_else(bad)?;
    if changes.contains_key("authMode")
        && !profile["securityOptions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v["value"] == encryption))
    {
        return Err(PanelError::new("WIFI_SECURITY_UNSUPPORTED", 422));
    }
    if changes.contains_key("authMode") && !changes.contains_key("password") {
        let key = snapshot["wireless"]["values"][id]["key"]
            .as_str()
            .unwrap_or("");
        if !(8..=63).contains(&key.len())
            && !(["psk2", "psk2+ccmp"].contains(&encryption)
                && key.len() == 64
                && key.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(PanelError::new("WIFI_PASSWORD_REQUIRED", 422));
        }
    }
    let legacy_mode = changes
        .get("bandwidth")
        .and_then(Value::as_str)
        .map(|s| s.trim_end_matches(|c: char| c.is_ascii_digit()));
    let mode = if let Some(mode) = changes.get("mode") {
        mode.as_str().ok_or_else(bad)?
    } else {
        legacy_mode.unwrap_or(profile["mode"].as_str().unwrap_or(""))
    };
    if changes.contains_key("mode")
        && !profile["modeOptions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v["value"] == mode))
    {
        return Err(PanelError::new("WIFI_MODE_UNSUPPORTED", 422));
    }
    let width = changes
        .get("bandwidth")
        .unwrap_or(&profile["bandwidth"])
        .as_str()
        .unwrap_or("");
    if (changes.contains_key("mode") || changes.contains_key("bandwidth"))
        && !profile["bandwidthOptions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v["value"] == width && v["mode"] == mode))
    {
        return Err(PanelError::new("WIFI_BANDWIDTH_UNSUPPORTED", 422));
    }
    if (changes.contains_key("mode")
        || changes.contains_key("bandwidth")
        || changes.contains_key("authMode"))
        && mode == "EHT"
        && !["sae", "sae-mixed"].contains(&encryption)
    {
        return Err(PanelError::new("WIFI7_SECURITY_REQUIRED", 422));
    }
    let mut out = serde_json::Map::new();
    for (key, value) in changes {
        let (target, val) = match key.as_str() {
            "ssid" => {
                let s = value
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 32 && !s.chars().any(char::is_control))
                    .ok_or_else(bad)?;
                ("ssid", json!(s))
            }
            "password" => {
                let s = value
                    .as_str()
                    .filter(|s| {
                        (8..=63).contains(&s.len()) && s.bytes().all(|b| (32..=126).contains(&b))
                            || ["psk2", "psk2+ccmp"].contains(&encryption)
                                && s.len() == 64
                                && s.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                    .ok_or_else(bad)?;
                ("key", json!(s))
            }
            "enabled" => (
                "disabled",
                json!(if value.as_bool().ok_or_else(bad)? {
                    "0"
                } else {
                    "1"
                }),
            ),
            "authMode" => ("encryption", json!(encryption)),
            "mode" => (".radioHtmode", json!(width)),
            "channel" => {
                let s = value.as_str().ok_or_else(bad)?;
                if !profile["channelOptions"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v["value"] == s))
                {
                    return Err(PanelError::new("WIFI_CHANNEL_UNSUPPORTED", 422));
                }
                (".radioChannel", json!(s))
            }
            "bandwidth" => {
                let s = value.as_str().ok_or_else(bad)?;
                if !profile["bandwidthOptions"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v["value"] == s))
                {
                    return Err(PanelError::new("WIFI_BANDWIDTH_UNSUPPORTED", 422));
                }
                (".radioHtmode", json!(s))
            }
            _ => return Err(bad()),
        };
        out.insert(target.into(), val);
    }
    if changes.get("enabled") == Some(&json!(true)) {
        out.insert(".radioDisabled".into(), json!("0"));
    }
    if changes.contains_key("authMode") {
        out.insert(
            "ieee80211w".into(),
            json!(match encryption {
                "sae" => "2",
                "sae-mixed" => "1",
                _ => "0",
            }),
        );
    }
    Ok(json!({"profile":profile,"changes":out}))
}
pub fn editable_profiles(snapshot: &Value) -> Vec<Value> {
    let mut rows = profiles(&snapshot["wireless"]);
    for row in &mut rows {
        let cap = &snapshot["radioCapabilities"][row["radio"].as_str().unwrap_or("")];
        let band = if row["band"] == "2g" { 2 } else { 5 };
        let mut channels = BTreeSet::new();
        for f in cap["frequencies"]["results"]
            .as_array()
            .into_iter()
            .flatten()
            .take(256)
        {
            if f["restricted"] != false || f["band"].as_u64().is_some_and(|b| b != band) {
                continue;
            }
            if let Some(n) = f["channel"].as_u64().filter(|n| {
                if band == 2 {
                    (1..=14).contains(n)
                } else {
                    (32..=177).contains(n)
                }
            }) {
                channels.insert(n);
            }
        }
        let mut options: Vec<Value> = channels
            .iter()
            .rev()
            .map(|n| json!({"value":n.to_string(),"label":format!("信道 {n}")}))
            .collect();
        if !options.is_empty() {
            options.insert(0, json!({"value":"auto","label":"自动"}));
        }
        let mut modes = BTreeSet::new();
        for m in cap["info"]["htmodes"]
            .as_array()
            .into_iter()
            .flatten()
            .take(32)
            .filter_map(Value::as_str)
        {
            if [
                "HT20", "HT40", "VHT20", "VHT40", "VHT80", "VHT160", "HE20", "HE40", "HE80",
                "HE160", "EHT20", "EHT40", "EHT80", "EHT160",
            ]
            .contains(&m)
                && (band == 5 || m.ends_with("20") || m.ends_with("40"))
            {
                modes.insert(m);
            }
        }
        let mut generations = BTreeSet::new();
        let mut widths: Vec<Value> = modes.into_iter().map(|m| {
            let mode = m.trim_end_matches(|c:char| c.is_ascii_digit());
            generations.insert(mode);
            json!({"value":m,"mode":mode,"label":format!("{} MHz",m.trim_start_matches(|c:char| c.is_ascii_alphabetic()))})
        }).collect();
        let rank = |m: &str| match m {
            "EHT" => 7,
            "HE" => 6,
            "VHT" => 5,
            "HT" => 4,
            _ => 0,
        };
        widths.sort_by_key(|v| {
            std::cmp::Reverse((
                rank(v["mode"].as_str().unwrap_or("")),
                v["label"]
                    .as_str()
                    .unwrap_or("")
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0),
            ))
        });
        row["mode"] = json!(
            row["channelWidth"]
                .as_str()
                .unwrap_or("")
                .trim_end_matches(|c: char| c.is_ascii_digit())
        );
        let mut generations: Vec<_> = generations.into_iter().collect();
        generations.sort_by_key(|m| std::cmp::Reverse(rank(m)));
        row["modeOptions"] = json!(generations.into_iter().map(|m| json!({"value":m,"label":match m {"HT"=>"WiFi 4","VHT"=>"WiFi 5","HE"=>"WiFi 6","EHT"=>"WiFi 7",_=>m}})).collect::<Vec<_>>());
        row["modeSupported"] = json!(!widths.is_empty());
        let mut security = vec![
            json!({"value":"psk2","label":"WPA2"}),
            json!({"value":"psk2+ccmp","label":"WPA2 · AES"}),
        ];
        if cap["saeSupported"] == true {
            security.extend([
                json!({"value":"sae-mixed","label":"WPA2 / WPA3"}),
                json!({"value":"sae","label":"WPA3"}),
            ]);
        }
        let current = row["authMode"].as_str().unwrap_or("");
        if !security.iter().any(|v| v["value"] == current) {
            security.push(
                json!({"value":current,"label":if current=="sae" {"WPA3"} else {"WPA2 / WPA3"}}),
            );
        }
        row["securitySupported"] = json!(true);
        security.sort_by_key(|v| {
            std::cmp::Reverse(match v["value"].as_str().unwrap_or("") {
                "sae" => 4,
                "sae-mixed" => 3,
                "psk2+ccmp" => 2,
                "psk2" => 1,
                _ => 0,
            })
        });
        row["securityOptions"] = json!(security);
        row["channelSupported"] = json!(!options.is_empty());
        row["channelOptions"] = json!(options);
        row["bandwidthSupported"] = json!(!widths.is_empty());
        row["bandwidthOptions"] = json!(widths);
        row["bandwidth"] = row["channelWidth"].clone();
    }
    rows
}
pub fn guard_supported(s: &Value) -> bool {
    s["nftAvailable"] == true
        && s["flowOffload"] == false
        && s["lanDevice"].as_str().is_some_and(interface)
        && s["wanDevice"].as_str().is_some_and(interface)
        && s["lanDevice"] != s["wanDevice"]
}
pub fn guard_script(snapshot: &Value, blocked: &BTreeSet<String>, cutoff: bool) -> Result<String> {
    if !guard_supported(snapshot) || blocked.len() > 128 || blocked.iter().any(|s| !mac(s)) {
        return Err(PanelError::new("FORWARD_GUARD_UNSUPPORTED", 422));
    }
    let lan = snapshot["lanDevice"].as_str().ok_or_else(bad)?;
    let wan = snapshot["wanDevice"].as_str().ok_or_else(bad)?;
    let mut script = String::from(
        "add table inet wlmb_guard { comment \"wlmb-server/1\"; }\nflush table inet wlmb_guard\nadd chain inet wlmb_guard forward { type filter hook forward priority -5; policy accept; }\n",
    );
    let route = format!("iifname \"{lan}\" oifname \"{wan}\"");
    if cutoff {
        script += &format!(
            "add rule inet wlmb_guard forward {route} counter drop comment \"wlmb-budget\"\n"
        );
    }
    for m in blocked {
        script += &format!(
            "add rule inet wlmb_guard forward {route} ether saddr {m} counter drop comment \"wlmb-device\"\n"
        );
    }
    Ok(script)
}
pub trait HostIo: Send + Sync {
    fn snapshot(&self) -> Result<Value>;
    fn save(&self, name: &str, raw: &[u8]) -> Result<()>;
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>>;
    fn wifi(&self, snapshot: &Value, changes: &[Value]) -> Result<()>;
    fn guard(&self, snapshot: &Value, blocked: &BTreeSet<String>, cutoff: bool) -> Result<()>;
    fn clear_guard(&self) -> Result<()>;
    fn known_device(&self, snapshot: &Value, address: &str) -> Result<bool> {
        Ok(snapshot["devices"].as_array().is_some_and(|a| {
            a.iter().any(|d| {
                d["mac"]
                    .as_str()
                    .is_some_and(|m| m.eq_ignore_ascii_case(address))
            })
        }))
    }
    fn guard_present(&self) -> Result<bool> {
        Ok(false)
    }
}
struct State {
    account: Accounting,
    blocked: BTreeSet<String>,
    snapshot: Value,
    apply_error: bool,
    cutoff_active: bool,
    saved: Instant,
    guard_revision: String,
    unify_enabled: bool,
    unify_managed: bool,
    observed: Option<Instant>,
    guard_checked: Instant,
}
pub struct HostControls {
    io: Arc<dyn HostIo>,
    security: Arc<Mutex<Security>>,
    state: Mutex<State>,
    gate: Mutex<()>,
    born: Instant,
}
impl HostControls {
    pub fn new(
        io: Arc<dyn HostIo>,
        security: Arc<Mutex<Security>>,
        born: Instant,
    ) -> Result<Arc<Self>> {
        let account = match io.load("traffic.json")? {
            Some(raw) => Accounting::restore(&raw)?,
            None => Accounting::default(),
        };
        let mut blocked = BTreeSet::new();
        if let Some(raw) = io.load("host.json")? {
            let v: Value = serde_json::from_slice(&raw).map_err(|_| bad())?;
            if v["version"] != 1 {
                return Err(bad());
            }
            for m in v["blocked"]
                .as_array()
                .filter(|a| a.len() <= 128)
                .ok_or_else(bad)?
            {
                let m = m.as_str().filter(|m| mac(m)).ok_or_else(bad)?;
                blocked.insert(m.to_ascii_lowercase());
            }
        }
        Ok(Arc::new(Self {
            io,
            security,
            state: Mutex::new(State {
                account,
                blocked,
                snapshot: Value::Null,
                apply_error: false,
                cutoff_active: false,
                saved: Instant::now(),
                guard_revision: String::new(),
                unify_enabled: false,
                unify_managed: false,
                observed: None,
                guard_checked: Instant::now(),
            }),
            gate: Mutex::new(()),
            born,
        }))
    }
    pub fn public(&self) -> Result<Value> {
        let s = self.state.lock().map_err(|_| changed())?;
        let stale = s.observed.is_none_or(|at| at.elapsed().as_secs() > 20);
        let p = if stale {
            vec![]
        } else {
            editable_profiles(&s.snapshot)
        };
        let unify = p.iter().any(|r| r["band"] == "2g") && p.iter().any(|r| r["band"] == "5g");
        let mut limit = s.account.limit();
        let mut traffic = s.account.public();
        traffic["stale"] = json!(stale);
        if stale {
            traffic["todayBytes"] = Value::Null;
            traffic["monthBytes"] = Value::Null;
        }
        limit["cutoffSupported"] = json!(!stale && guard_supported(&s.snapshot));
        limit["cutoffActive"] = json!(s.cutoff_active && !stale && !s.apply_error);
        limit["applyError"] = json!(s.apply_error || stale);
        Ok(
            json!({"traffic":traffic,"trafficLimit":limit,"blocked":s.blocked,"blockApplyError":s.apply_error||stale,"deviceBlockSupported":!stale&&guard_supported(&s.snapshot),
            "wifiControls":{"profiles":p,"editableBands":p.iter().map(|p|p["band"].clone()).collect::<Vec<_>>(),"bandUnifySupported":unify,
                "bandUnifyEnabled":s.unify_enabled&&!stale,"bandUnifyStateKnown":!stale&&wifi_unified(&s.snapshot).is_some(),"bandUnifyManaged":s.unify_managed&&!stale,"bandUnifySource":if s.unify_managed{"plugin"}else{"system-config"},"stale":stale}}),
        )
    }
    pub fn sample(&self, date: Option<Date>, counters: Option<(&str, u64, u64)>) -> Result<()> {
        let _gate = self
            .gate
            .try_lock()
            .map_err(|_| PanelError::new("HOST_BUSY", 409))?;
        let snapshot = self.io.snapshot()?;
        let unify_managed = self.io.load("wifi-backup.json")?.is_some_and(|raw| {
            serde_json::from_slice::<Value>(&raw).is_ok_and(|b| b["after"] == snapshot["wireless"])
        });
        let mut s = self.state.lock().map_err(|_| changed())?;
        s.snapshot = snapshot;
        s.observed = Some(Instant::now());
        s.unify_enabled = wifi_unified(&s.snapshot) == Some(true);
        s.unify_managed = unify_managed;
        s.account.sample(date, counters)?;
        let cutoff = s.account.limit()["limitReached"] == true && s.account.budget.cutoff;
        let revision = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&json!([
                    s.snapshot["lanDevice"],
                    s.snapshot["wanDevice"],
                    s.snapshot["flowOffload"],
                    s.blocked,
                    cutoff
                ]))
                .map_err(|_| bad())?
            )
        );
        let check = (cutoff || !s.blocked.is_empty()) && s.guard_checked.elapsed().as_secs() >= 30;
        if check {
            s.guard_checked = Instant::now();
        }
        let apply = revision != s.guard_revision || s.apply_error;
        let snapshot = s.snapshot.clone();
        let blocked = s.blocked.clone();
        if apply {
            s.apply_error = true;
            s.cutoff_active = false;
        }
        let checkpoint = if s.saved.elapsed().as_secs() >= 900 {
            Some(s.account.checkpoint()?)
        } else {
            None
        };
        drop(s);
        let missing = check && !self.io.guard_present().unwrap_or(false);
        if apply || missing {
            let outcome = if cutoff || !blocked.is_empty() {
                self.io.guard(&snapshot, &blocked, cutoff)
            } else {
                self.io.clear_guard()
            };
            let mut s = self.state.lock().map_err(|_| changed())?;
            s.apply_error = outcome.is_err();
            s.cutoff_active = cutoff && outcome.is_ok();
            if outcome.is_ok() {
                s.guard_revision = revision;
            }
        }
        if let Some(raw) = checkpoint {
            let ok = self.io.save("traffic.json", &raw).is_ok();
            let mut s = self.state.lock().map_err(|_| changed())?;
            s.account.persistence_ok = ok;
            s.saved = Instant::now();
        }
        Ok(())
    }
    fn validate(&self, snapshot: &Value, op: &Value) -> Result<()> {
        match op["kind"].as_str() {
            Some("wifi_edit") => {
                wifi_changes(snapshot, op)?;
            }
            Some("wifi_unify") => {
                if !exact(op, &["kind", "enabled"]) || op["enabled"].as_bool().is_none() {
                    return Err(bad());
                }
                let p = profiles(&snapshot["wireless"]);
                if !p.iter().any(|r| r["band"] == "2g") || !p.iter().any(|r| r["band"] == "5g") {
                    return Err(PanelError::new("WIFI_UNIFY_UNSUPPORTED", 422));
                }
                if op["enabled"] == false {
                    let managed = self
                        .io
                        .load("wifi-backup.json")?
                        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                        .is_some_and(|backup| backup["after"] == snapshot["wireless"]);
                    if !managed {
                        if wifi_unified(snapshot) != Some(true) {
                            return Err(PanelError::new("WIFI_BACKUP_CHANGED", 409));
                        }
                        let primary = p.iter().find(|r| r["band"] == "2g").ok_or_else(bad)?;
                        wifi_changes(
                            snapshot,
                            &json!({"kind":"wifi_edit","band":"5g","changes":{"ssid":separated_ssid(primary["ssid"].as_str().ok_or_else(bad)?)}}),
                        )?;
                    }
                } else {
                    let primary = p.iter().find(|r| r["band"] == "2g").ok_or_else(bad)?;
                    let row =
                        &snapshot["wireless"]["values"][primary["id"].as_str().ok_or_else(bad)?];
                    wifi_changes(
                        snapshot,
                        &json!({"kind":"wifi_edit","band":"2g","changes":{"ssid":row["ssid"],"password":row["key"]}}),
                    )?;
                    wifi_changes(
                        snapshot,
                        &json!({"kind":"wifi_edit","band":"5g","changes":{"ssid":row["ssid"],"password":row["key"],"authMode":row["encryption"]}}),
                    )?;
                    let secondary = p.iter().find(|r| r["band"] == "5g").ok_or_else(bad)?;
                    let dst = &snapshot["wireless"]["values"]
                        [secondary["id"].as_str().ok_or_else(bad)?];
                    if row["encryption"] != dst["encryption"]
                        && ["sae", "sae-mixed"].iter().any(|v| row["encryption"] == *v)
                        && dst["ieee80211w"] == "0"
                    {
                        return Err(PanelError::new("WIFI_UNIFY_SECURITY_REQUIRED", 422));
                    }
                }
            }
            Some("traffic_limit") => {
                if !exact(
                    op,
                    &[
                        "kind",
                        "enabled",
                        "limitBytes",
                        "startDay",
                        "threshold",
                        "cutoffEnabled",
                    ],
                ) {
                    return Err(bad());
                }
                let b = Budget::decode(op)?;
                if b.cutoff && !guard_supported(snapshot) {
                    return Err(PanelError::new("FORWARD_GUARD_UNSUPPORTED", 422));
                }
            }
            Some("device_block") => {
                if !exact(op, &["kind", "mac", "blocked"])
                    || !op["mac"].as_str().is_some_and(mac)
                    || op["blocked"].as_bool().is_none()
                {
                    return Err(bad());
                }
                let m = op["mac"].as_str().unwrap_or("").to_ascii_lowercase();
                let s = self.state.lock().map_err(|_| changed())?;
                let saved = s.blocked.contains(&m);
                let full = s.blocked.len() >= 128;
                drop(s);
                if op["blocked"] == true && !guard_supported(snapshot) {
                    return Err(PanelError::new("FORWARD_GUARD_UNSUPPORTED", 422));
                }
                if !saved && (full || !self.io.known_device(snapshot, &m)?) {
                    return Err(PanelError::new("DEVICE_NOT_FOUND", 422));
                }
            }
            _ => return Err(PanelError::new("HOST_UNSUPPORTED", 422)),
        }
        Ok(())
    }
    fn revision(&self, snapshot: &Value, op: &Value) -> Result<String> {
        let s = self.state.lock().map_err(|_| changed())?;
        let raw = serde_json::to_vec(&json!([
            snapshot["wireless"],
            snapshot["lanDevice"],
            snapshot["wanDevice"],
            snapshot["flowOffload"],
            snapshot["nftAvailable"],
            snapshot["network"],
            s.account.budget.public(0, false),
            s.blocked,
            op
        ]))
        .map_err(|_| bad())?;
        Ok(format!("{:x}", Sha256::digest(raw)))
    }
    pub fn prepare(&self, op: &Value, owner: &str) -> Result<Value> {
        let _gate = self
            .gate
            .try_lock()
            .map_err(|_| PanelError::new("HOST_BUSY", 409))?;
        let snapshot = self.io.snapshot()?;
        self.validate(&snapshot, op)?;
        let revision = self.revision(&snapshot, op)?;
        let mut c = self.security.lock().map_err(|_| changed())?.prepare(
            owner,
            "host",
            &format!("host:{revision}"),
            self.born.elapsed(),
        )?;
        c["revision"] = json!(revision);
        c["warning"] = json!(match op["kind"].as_str() {
            Some("wifi_unify")
                if op["enabled"] == false
                    && !self.state.lock().map_err(|_| changed())?.unify_managed =>
                "当前为系统配置的同名 WiFi，未保存旧名称。关闭后仅将 5G 名称加上 _5G，密码不变；WiFi 会短暂中断，不修改拨号或有线网络。",
            Some("wifi_edit" | "wifi_unify") =>
                "将保存无线设置并重新加载配置，WiFi 连接可能中断。不会修改拨号或有线网络配置。",
            Some("device_block") =>
                "仅限制这台设备的 LAN 到 WAN 转发，管理地址仍可访问。不支持绕过系统加速；停止插件后解除限制。",
            _ =>
                "按插件启用后的上下行合计统计，不等同运营商账单。更改起始日会重新开始本周期；启用停网将保留 LAN 管理。",
        });
        Ok(c)
    }
    pub fn execute(&self, op: &Value, revision: &str, nonce: &str, owner: &str) -> Result<Value> {
        self.security.lock().map_err(|_| changed())?.consume(
            nonce,
            owner,
            "host",
            &format!("host:{revision}"),
            self.born.elapsed(),
        )?;
        let _gate = self
            .gate
            .try_lock()
            .map_err(|_| PanelError::new("HOST_BUSY", 409))?;
        let snapshot = self.io.snapshot()?;
        self.validate(&snapshot, op)?;
        if revision != self.revision(&snapshot, op)? {
            return Err(changed());
        }
        match op["kind"].as_str() {
            Some("wifi_edit") => {
                self.io.wifi(&snapshot, &[wifi_changes(&snapshot, op)?])?;
            }
            Some("wifi_unify") => {
                let p = profiles(&snapshot["wireless"]);
                let primary = p.iter().find(|r| r["band"] == "2g").ok_or_else(bad)?;
                let secondary = p.iter().find(|r| r["band"] == "5g").ok_or_else(bad)?;
                if op["enabled"] == true {
                    if wifi_unified(&snapshot) == Some(true)
                        || self.io.load("wifi-backup.json")?.is_some_and(|raw| {
                            serde_json::from_slice::<Value>(&raw)
                                .is_ok_and(|b| b["after"] == snapshot["wireless"])
                        })
                    {
                        return Ok(json!({"accepted":true,"unchanged":true}));
                    }
                    let src =
                        &snapshot["wireless"]["values"][primary["id"].as_str().ok_or_else(bad)?];
                    let changes =
                        json!({"ssid":src["ssid"],"key":src["key"],"encryption":src["encryption"]});
                    let mut after = snapshot["wireless"].clone();
                    for k in ["ssid", "key", "encryption"] {
                        after["values"][secondary["id"].as_str().ok_or_else(bad)?][k] =
                            changes[k].clone();
                    }
                    self.io.save(
                        "wifi-backup.json",
                        &serde_json::to_vec(
                            &json!({"version":1,"before":snapshot["wireless"],"after":after}),
                        )
                        .map_err(|_| bad())?,
                    )?;
                    self.io
                        .wifi(&snapshot, &[json!({"profile":secondary,"changes":changes})])?;
                } else {
                    let backup = self
                        .io
                        .load("wifi-backup.json")?
                        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                        .filter(|b| b["after"] == snapshot["wireless"]);
                    let mut rows = vec![];
                    if let Some(backup) = backup {
                        for profile in p {
                            let before = &backup["before"]["values"]
                                [profile["id"].as_str().ok_or_else(bad)?];
                            rows.push(json!({"profile":profile,"changes":{"ssid":before["ssid"],"key":before["key"],"encryption":before["encryption"]}}));
                        }
                    } else {
                        rows.push(wifi_changes(&snapshot,&json!({"kind":"wifi_edit","band":"5g","changes":{"ssid":separated_ssid(primary["ssid"].as_str().ok_or_else(bad)?)}}))?);
                    }
                    self.io.wifi(&snapshot, &rows)?;
                    self.io.save("wifi-backup.json", b"{}")?;
                }
            }
            Some("traffic_limit") => {
                let mut next = self.state.lock().map_err(|_| changed())?.account.clone();
                next.configure(Budget::decode(op)?);
                self.io.save("traffic.json", &next.checkpoint()?)?;
                next.persistence_ok = true;
                let mut s = self.state.lock().map_err(|_| changed())?;
                s.account = next;
                s.saved = Instant::now();
                s.guard_revision.clear();
            }
            Some("device_block") => {
                let m = op["mac"].as_str().ok_or_else(bad)?.to_ascii_lowercase();
                let mut next = self.state.lock().map_err(|_| changed())?.blocked.clone();
                if op["blocked"] == true {
                    next.insert(m);
                } else {
                    next.remove(&m);
                }
                self.io.save(
                    "host.json",
                    &serde_json::to_vec(&json!({"version":1,"blocked":next})).map_err(|_| bad())?,
                )?;
                let mut s = self.state.lock().map_err(|_| changed())?;
                s.blocked = next;
                s.guard_revision.clear();
            }
            _ => return Err(bad()),
        }
        if ["device_block", "traffic_limit"].contains(&op["kind"].as_str().unwrap_or("")) {
            let mut s = self.state.lock().map_err(|_| changed())?;
            let cutoff = s.account.limit()["limitReached"] == true && s.account.budget.cutoff;
            let blocked = s.blocked.clone();
            s.apply_error = true;
            s.cutoff_active = false;
            drop(s);
            let outcome = if cutoff || !blocked.is_empty() {
                self.io.guard(&snapshot, &blocked, cutoff)
            } else {
                self.io.clear_guard()
            };
            let mut s = self.state.lock().map_err(|_| changed())?;
            s.apply_error = outcome.is_err();
            s.cutoff_active = cutoff && outcome.is_ok();
            outcome?;
        }
        Ok(
            json!({"accepted":true,"saved":true,"note":"设置已保存；无线、转发和流量状态请刷新后核对，不自动重试。"}),
        )
    }
    pub fn shutdown(&self) {
        let Ok(_gate) = self.gate.lock() else { return };
        let checkpoint = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.account.checkpoint().ok());
        if let Some(raw) = checkpoint {
            let _ = self.io.save("traffic.json", &raw);
        }
        let _ = self.io.clear_guard();
    }
}
