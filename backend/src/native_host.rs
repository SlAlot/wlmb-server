//! OpenWrt原生主机数据与配置；只修改本插件核验过的对象，不依赖模组厂商。
use crate::{
    PanelError, Result,
    host_controls::{self, HostIo},
    private_store::PrivateStore,
    transport::{self, Bus},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub struct NativeHost {
    bus: Arc<dyn Bus>,
    store: PrivateStore,
    firewall: Mutex<Option<(Instant, bool)>>,
    radio_cache: Mutex<Option<RadioCache>>,
    feature_cache: Mutex<Option<(Instant, Value)>>,
}
struct RadioCache {
    wireless: Value,
    observed: Instant,
    capabilities: Value,
    ttl: Duration,
}
fn wifi_features(fresh: Value, cache: &mut Option<(Instant, Value)>) -> (Value, bool) {
    if fresh["hostapd"]["sae"].is_boolean()
        || fresh["wifi"].is_boolean() && fresh.get("hostapd").is_none()
    {
        *cache = Some((Instant::now(), fresh.clone()));
        return (fresh, true);
    }
    let recent = cache
        .as_ref()
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(300));
    (
        recent
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Null),
        false,
    )
}
fn bad() -> PanelError {
    PanelError::new("HOST_SYSTEM_UNAVAILABLE", 503)
}
impl NativeHost {
    pub fn new(bus: Arc<dyn Bus>, store: PrivateStore) -> Arc<Self> {
        Arc::new(Self {
            bus,
            store,
            firewall: Mutex::new(None),
            radio_cache: Mutex::new(None),
            feature_cache: Mutex::new(None),
        })
    }
    fn get(&self, config: &str) -> Result<Value> {
        self.bus.call(
            "uci",
            "get",
            json!({"config":config}),
            Duration::from_secs(2),
        )
    }
    fn nft(args: &[&str], input: Option<&[u8]>) -> Result<String> {
        let program = if Path::new("/usr/sbin/nft").is_file() {
            "/usr/sbin/nft"
        } else {
            "/sbin/nft"
        };
        transport::process(program, args, input, Duration::from_secs(3))
    }
    fn nft_snapshot() -> Result<Value> {
        serde_json::from_str(&Self::nft(&["-j", "list", "ruleset"], None)?).map_err(|_| bad())
    }
    fn owned(rules: &Value) -> Result<bool> {
        let rows = rules["nftables"].as_array().ok_or_else(bad)?;
        for row in rows {
            let table = &row["table"];
            if table["family"] == "inet" && table["name"] == "wlmb_guard" {
                if table["comment"] != "wlmb-server/1" {
                    return Err(PanelError::new("GUARD_TABLE_CONFLICT", 409));
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn flow(rules: &Value) -> Result<bool> {
        Ok(rules["nftables"]
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .any(|r| r.get("flowtable").is_some()))
    }
    fn clean_wireless() -> Result<()> {
        if !transport::process(
            "/sbin/uci",
            &["-q", "changes", "wireless"],
            None,
            Duration::from_secs(2),
        )?
        .trim()
        .is_empty()
        {
            return Err(PanelError::new("WIFI_PENDING_CHANGES", 409));
        }
        Ok(())
    }
    fn radio_capabilities(&self, wireless: &Value) -> Value {
        let Ok(mut cache) = self.radio_cache.lock() else {
            return Value::Null;
        };
        if let Some(old) = &*cache {
            if old.wireless == *wireless && old.observed.elapsed() < old.ttl {
                return old.capabilities.clone();
            }
        }
        let status = self
            .bus
            .call(
                "network.wireless",
                "status",
                json!({}),
                Duration::from_secs(1),
            )
            .unwrap_or(Value::Null);
        let fresh_features = self
            .bus
            .call("luci", "getFeatures", json!({}), Duration::from_secs(1))
            .unwrap_or(Value::Null);
        let (features, feature_fresh) = self
            .feature_cache
            .lock()
            .map(|mut old| wifi_features(fresh_features, &mut old))
            .unwrap_or((Value::Null, false));
        let mut complete = feature_fresh && status.is_object();
        let mut out = json!({});
        for p in host_controls::profiles(wireless).into_iter().take(2) {
            let Some(radio) = p["radio"].as_str() else {
                continue;
            };
            let Some(iface) = status[radio]["interfaces"]
                .as_array()
                .and_then(|a| a.iter().find(|i| i["section"] == p["id"]))
            else {
                complete = false;
                continue;
            };
            let Some(name) = iface["ifname"]
                .as_str()
                .filter(|s| host_controls::interface(s))
            else {
                complete = false;
                continue;
            };
            let info = self
                .bus
                .call(
                    "iwinfo",
                    "info",
                    json!({"device":name}),
                    Duration::from_secs(1),
                )
                .unwrap_or(Value::Null);
            let frequencies = self
                .bus
                .call(
                    "iwinfo",
                    "freqlist",
                    json!({"device":name}),
                    Duration::from_secs(1),
                )
                .unwrap_or(Value::Null);
            complete &= info["htmodes"].is_array() && frequencies["results"].is_array();
            out[radio] = json!({"info":{"htmodes":info["htmodes"]},"frequencies":frequencies,"saeSupported":features["hostapd"]["sae"]==true});
        }
        let ttl = Duration::from_secs(if complete { 60 } else { 5 });
        *cache = Some(RadioCache {
            wireless: wireless.clone(),
            observed: Instant::now(),
            capabilities: out.clone(),
            ttl,
        });
        out
    }
    fn clean_network() -> Result<()> {
        if !transport::process(
            "/sbin/uci",
            &["-q", "changes", "network"],
            None,
            Duration::from_secs(2),
        )?
        .trim()
        .is_empty()
        {
            return Err(PanelError::new("WIFI_PENDING_CHANGES", 409));
        }
        Ok(())
    }
}

impl HostIo for NativeHost {
    fn known_device(&self, snapshot: &Value, address: &str) -> Result<bool> {
        if !host_controls::mac(address) {
            return Err(bad());
        }
        if snapshot["devices"].as_array().is_some_and(|a| {
            a.iter().any(|d| {
                d["mac"]
                    .as_str()
                    .is_some_and(|m| m.eq_ignore_ascii_case(address))
            })
        }) {
            return Ok(true);
        }
        let until = Instant::now() + Duration::from_secs(3);
        let call = |object, method, params| {
            until
                .checked_duration_since(Instant::now())
                .and_then(|left| {
                    self.bus
                        .call(object, method, params, left.min(Duration::from_secs(1)))
                        .ok()
                })
        };
        let status = call("network.wireless", "status", json!({})).unwrap_or(Value::Null);
        for radio in status
            .as_object()
            .into_iter()
            .flat_map(|r| r.values())
            .take(4)
        {
            if radio["up"] != true {
                continue;
            }
            for iface in radio["interfaces"].as_array().into_iter().flatten().take(4) {
                let cfg = &iface["config"];
                let lan = cfg["network"] == "lan"
                    || cfg["network"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|n| n == "lan"));
                if cfg["mode"] != "ap" || !lan || !crate::lan::ap_enabled(radio, cfg) {
                    continue;
                }
                let Some(name) = iface["ifname"]
                    .as_str()
                    .filter(|n| host_controls::interface(n))
                else {
                    continue;
                };
                let stations =
                    call("iwinfo", "assoclist", json!({"device":name})).unwrap_or(Value::Null);
                if stations["results"].as_array().is_some_and(|a| {
                    a.iter().take(128).any(|row| {
                        row["mac"]
                            .as_str()
                            .is_some_and(|m| m.eq_ignore_ascii_case(address))
                    })
                }) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.store.load(name)
    }
    fn save(&self, name: &str, raw: &[u8]) -> Result<()> {
        self.store.save(name, raw)
    }
    fn snapshot(&self) -> Result<Value> {
        let wireless = self.get("wireless").unwrap_or(Value::Null);
        let network = self.get("network").unwrap_or(Value::Null);
        let firewall = self.get("firewall").unwrap_or(Value::Null);
        let lan = self.bus.call(
            "network.interface.lan",
            "status",
            json!({}),
            Duration::from_secs(2),
        )?;
        let wan = self.bus.call(
            "network.interface",
            "dump",
            json!({}),
            Duration::from_secs(2),
        )?;
        let lan_device = lan["l3_device"]
            .as_str()
            .filter(|s| host_controls::interface(s))
            .ok_or_else(bad)?;
        let mut flow = true;
        let mut available = false;
        if let Ok(mut cache) = self.firewall.lock() {
            if let Some((at, has_flow)) = cache
                .as_ref()
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
            {
                let _ = at;
                flow = *has_flow;
                available = true;
            } else if let Ok(rules) = Self::nft_snapshot() {
                flow = Self::flow(&rules)?;
                available = true;
                *cache = Some((Instant::now(), flow));
            }
        }
        let configured_flow = firewall["values"].as_object().is_none_or(|m| {
            m.values().any(|r| {
                r[".type"] == "defaults"
                    && (r["flow_offloading"] == "1" || r["flow_offloading_hw"] == "1")
            })
        });
        let arp = transport::bounded_read(Path::new("/proc/net/arp"), 65536).unwrap_or_default();
        let leases =
            transport::bounded_read(Path::new("/tmp/dhcp.leases"), 65536).unwrap_or_default();
        let peers = crate::lan::peers(&arp, &leases, &[lan_device.into()]);
        let radio_capabilities = self.radio_capabilities(&wireless);
        Ok(
            json!({"wireless":wireless,"radioCapabilities":radio_capabilities,"network":network,"lanDevice":lan_device,"wanDevice":crate::telemetry::wan_device(&wan),"nftAvailable":available,
            "flowOffload":flow||configured_flow,"devices":peers.into_values().take(256).collect::<Vec<_>>()}),
        )
    }
    fn wifi(&self, snapshot: &Value, changes: &[Value]) -> Result<()> {
        if changes.is_empty() || changes.len() > 2 {
            return Err(bad());
        }
        Self::clean_wireless()?;
        Self::clean_network()?;
        if snapshot["network"].is_null() || self.get("network")? != snapshot["network"] {
            return Err(PanelError::new("HOST_CHANGED", 409));
        }
        if self.get("wireless")? != snapshot["wireless"] {
            return Err(PanelError::new("HOST_CHANGED", 409));
        }
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| bad())?;
        let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let dir = Path::new(crate::rpc::DIRECTORY).join(format!("uci-{suffix}"));
        let meta = fs::symlink_metadata(crate::rpc::DIRECTORY).map_err(|_| bad())?;
        if !meta.is_dir() || meta.uid() != 0 || meta.mode() & 0o077 != 0 {
            return Err(bad());
        }
        fs::create_dir(&dir).map_err(|_| bad())?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|_| bad())?;
        let dir_text = dir.to_str().ok_or_else(bad)?;
        let outcome = (|| {
            let mut batch = String::new();
            let mut verify = vec![];
            for change in changes {
                let id = change["profile"]["id"]
                    .as_str()
                    .filter(|s| host_controls::identifier(s))
                    .ok_or_else(bad)?;
                let radio = change["profile"]["radio"]
                    .as_str()
                    .filter(|s| host_controls::identifier(s))
                    .ok_or_else(bad)?;
                let rows = change["changes"].as_object().ok_or_else(bad)?;
                for (key, v) in rows {
                    if ![
                        "ssid",
                        "key",
                        "encryption",
                        "ieee80211w",
                        "disabled",
                        ".radioDisabled",
                        ".radioChannel",
                        ".radioHtmode",
                    ]
                    .contains(&key.as_str())
                    {
                        return Err(bad());
                    }
                    let val = v
                        .as_str()
                        .filter(|s| s.len() <= 128 && !s.chars().any(char::is_control))
                        .ok_or_else(bad)?;
                    let target = if key == ".radioDisabled" {
                        format!("wireless.{radio}.disabled")
                    } else if key == ".radioChannel" {
                        format!("wireless.{radio}.channel")
                    } else if key == ".radioHtmode" {
                        format!("wireless.{radio}.htmode")
                    } else {
                        format!("wireless.{id}.{key}")
                    };
                    batch += &format!("set {target}='{}'\n", val.replace('\'', "'\\''"));
                    verify.push((target, val.to_owned()));
                }
            }
            transport::process(
                "/sbin/uci",
                &["-q", "-t", dir_text, "batch"],
                Some(batch.as_bytes()),
                Duration::from_secs(3),
            )?;
            for (target, expected) in &verify {
                let got = transport::process(
                    "/sbin/uci",
                    &["-q", "-t", dir_text, "get", target],
                    None,
                    Duration::from_secs(2),
                )?;
                if got.trim_end_matches(['\n', '\r']) != expected {
                    return Err(PanelError::new("WIFI_STAGE_FAILED", 409));
                }
            }
            Self::clean_wireless()?;
            Self::clean_network()?;
            if self.get("network")? != snapshot["network"] {
                return Err(PanelError::new("HOST_CHANGED", 409));
            }
            if self.get("wireless")? != snapshot["wireless"] {
                return Err(PanelError::new("HOST_CHANGED", 409));
            }
            transport::process(
                "/sbin/uci",
                &["-q", "-t", dir_text, "commit", "wireless"],
                None,
                Duration::from_secs(3),
            )?;
            Self::clean_network()?;
            if self.get("network")? != snapshot["network"] {
                return Err(PanelError::new("WIFI_NETWORK_CHANGED", 409));
            }
            self.bus
                .call("network", "reload", json!({}), Duration::from_secs(3))
                .map_err(|_| PanelError::new("WIFI_RELOAD_UNKNOWN", 409))?;
            let actual = self.get("wireless")?;
            for (target, expected) in verify {
                let fields: Vec<_> = target.split('.').collect();
                if fields.len() != 3 || actual["values"][fields[1]][fields[2]] != expected {
                    return Err(PanelError::new("WIFI_READBACK_MISMATCH", 409));
                }
            }
            Ok(())
        })();
        let _ = fs::remove_file(dir.join("wireless"));
        let _ = fs::remove_dir(&dir);
        outcome
    }
    fn guard(&self, snapshot: &Value, blocked: &BTreeSet<String>, cutoff: bool) -> Result<()> {
        let rules = Self::nft_snapshot()?;
        Self::owned(&rules)?;
        if Self::flow(&rules)? {
            return Err(PanelError::new("FORWARD_GUARD_UNSUPPORTED", 422));
        }
        let script = host_controls::guard_script(snapshot, blocked, cutoff)?;
        Self::nft(&["-c", "-f", "-"], Some(script.as_bytes()))?;
        Self::nft(&["-f", "-"], Some(script.as_bytes()))?;
        if !Self::owned(&Self::nft_snapshot()?)? {
            return Err(PanelError::new("GUARD_APPLY_UNKNOWN", 409));
        }
        Ok(())
    }
    fn clear_guard(&self) -> Result<()> {
        if !Path::new("/usr/sbin/nft").is_file() && !Path::new("/sbin/nft").is_file() {
            return Ok(());
        }
        let rules = Self::nft_snapshot()?;
        if Self::owned(&rules)? {
            Self::nft(&["delete", "table", "inet", "wlmb_guard"], None)?;
        }
        Ok(())
    }
    fn guard_present(&self) -> Result<bool> {
        let rules: Value = serde_json::from_str(&Self::nft(&["-j", "list", "tables"], None)?)
            .map_err(|_| bad())?;
        Self::owned(&rules)
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn system_wifi_without_hostapd_keeps_basic_capabilities_without_guessing_sae() {
        let mut cache = None;
        let features = json!({"wifi":true,"opkg":true});
        let (actual, fresh) = wifi_features(features.clone(), &mut cache);
        assert!(fresh);
        assert_eq!(actual, features);
        assert_ne!(actual["hostapd"]["sae"], true);
    }
    #[test]
    fn transient_feature_failure_preserves_recent_success_without_renewing_it() {
        let mut old = None;
        assert_eq!(wifi_features(Value::Null, &mut old), (Value::Null, false));
        let supported = json!({"hostapd":{"sae":true}});
        assert_eq!(
            wifi_features(supported.clone(), &mut old),
            (supported.clone(), true)
        );
        let observed = old.as_ref().map(|(at, _)| *at);
        assert_eq!(
            wifi_features(json!({"error":"timeout"}), &mut old),
            (supported, false)
        );
        assert_eq!(old.as_ref().map(|(at, _)| *at), observed);
        let unsupported = json!({"hostapd":{"sae":false}});
        assert_eq!(
            wifi_features(unsupported.clone(), &mut old),
            (unsupported, true)
        );
    }
    #[test]
    fn expired_or_malformed_features_never_advertise_guessed_support() {
        let mut old = Some((
            Instant::now() - Duration::from_secs(301),
            json!({"hostapd":{"sae":true}}),
        ));
        let observed = old.as_ref().map(|(at, _)| *at);
        assert_eq!(
            wifi_features(json!({"hostapd":{"sae":"true"}}), &mut old),
            (Value::Null, false)
        );
        assert_eq!(old.as_ref().map(|(at, _)| *at), observed);
    }
}
