//! 无线运行态、租约和终端计数的统一快照；区分配置状态与实际在线。
use crate::transport::{Bus, bounded_read};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn mac(value: &str) -> Option<String> {
    crate::host_controls::mac(value).then(|| value.to_ascii_lowercase())
}
// 原厂未向 netifd 报告无线时，只从实际 AP 信息回退展示；不按接口或 phy 编号猜频段。
fn fallback_wireless(
    call: impl Fn(&str, &str, Value) -> Option<Value>,
) -> (Value, BTreeMap<String, Value>) {
    let mut status = json!({});
    let mut infos = BTreeMap::new();
    let devices = call("iwinfo", "devices", json!({})).unwrap_or(Value::Null);
    let Some(devices) = devices["devices"].as_array().filter(|v| v.len() <= 8) else {
        return (status, infos);
    };
    let config = call("uci", "get", json!({"config":"wireless"})).unwrap_or(Value::Null);
    let mut seen = std::collections::BTreeSet::new();
    for device in devices {
        let Some(ifname) = device.as_str().filter(|s| name(s)) else {
            continue;
        };
        if !seen.insert(ifname) {
            continue;
        }
        infos.insert(ifname.to_owned(), Value::Null);
        let Some(info) = call("iwinfo", "info", json!({"device":ifname})) else {
            continue;
        };
        infos.insert(ifname.to_owned(), info.clone());
        let band = match info["frequency"].as_u64() {
            Some(2400..=2500) => "2g",
            Some(4900..=5900) => "5g",
            Some(5925..=7125) => "6g",
            _ => continue,
        };
        let Some(ssid) = info["ssid"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 32)
        else {
            continue;
        };
        if info["mode"] != "Master" || info["channel"].as_u64().is_none_or(|n| n == 0 || n > 233) {
            continue;
        }
        let candidates: Vec<_> = config["values"]
            .as_object()
            .filter(|m| m.len() <= 64)
            .into_iter()
            .flat_map(|m| m.iter())
            .filter(|(_, row)| {
                let radio = row["device"].as_str().unwrap_or("");
                row[".type"] == "wifi-iface"
                    && row["mode"] == "ap"
                    && config["values"][radio]["band"] == band
                    && if let Some(explicit) = row["ifname"].as_str() {
                        explicit == ifname
                    } else {
                        row["ssid"] == ssid
                    }
            })
            .collect();
        let bound = candidates
            .first()
            .filter(|_| candidates.len() == 1)
            .filter(|(id, row)| {
                crate::host_controls::identifier(id)
                    && (row["network"] == "lan"
                        || row["network"]
                            .as_array()
                            .is_some_and(|v| v.len() == 1 && v[0] == "lan"))
            });
        let section = bound
            .map(|(id, _)| id.to_string())
            .unwrap_or_else(|| format!("iwinfo-{ifname}"));
        // 未核实 LAN 归属的 AP 仅展示运行状态，不采集为主网络终端或允许控制。
        let network = if bound.is_some() { "lan" } else { "unverified" };
        status[ifname] = json!({"up":true,"source":"iwinfo-fallback","interfaces":[{"section":section,"ifname":ifname,"config":{"mode":"ap","network":network,"ssid":ssid}}]});
        infos.insert(ifname.to_owned(), info);
    }
    (status, infos)
}
pub fn disabled(value: &Value) -> bool {
    crate::host_controls::wifi_disabled(value)
}
pub fn ap_state(radio: &Value, config: &Value) -> Option<bool> {
    if disabled(&radio["disabled"]) || disabled(&config["disabled"]) {
        return Some(false);
    }
    match &radio["up"] {
        Value::Bool(value) => Some(*value),
        value if value == 1 || value == "1" => Some(true),
        value if value == 0 || value == "0" => Some(false),
        _ => None,
    }
}
pub fn ap_enabled(radio: &Value, config: &Value) -> bool {
    ap_state(radio, config) == Some(true)
}
pub fn age_snapshot(value: &mut Value, now_ms: u64) {
    value["stale"] = json!(
        value["observedAtMs"]
            .as_u64()
            .is_none_or(|at| at > now_ms || now_ms - at > 30000)
    );
}
pub fn peers(arp: &str, leases: &str, lan_devices: &[String]) -> BTreeMap<String, Value> {
    let labels: BTreeMap<_, _> = leases
        .lines()
        .take(256)
        .filter_map(|line| {
            let f: Vec<_> = line.split_whitespace().collect();
            if f.len() < 4 {
                return None;
            }
            Some((
                mac(f[1])?,
                if f[3] == "*" {
                    String::new()
                } else {
                    f[3].chars().take(64).collect()
                },
            ))
        })
        .collect();
    let mut result = BTreeMap::new();
    for line in arp.lines().skip(1).take(256) {
        let f: Vec<_> = line.split_whitespace().collect();
        if f.len() != 6
            || !lan_devices.iter().any(|n| n == f[5])
            || f[0].parse::<std::net::Ipv4Addr>().is_err()
        {
            continue;
        }
        let flags = u32::from_str_radix(f[2].trim_start_matches("0x"), 16).unwrap_or(0);
        if flags & 2 == 0 {
            continue;
        }
        if let Some(mac) = mac(f[3]) {
            result.insert(mac.clone(), json!({"mac":mac,"ip":f[0],"name":labels.get(&mac).cloned().unwrap_or_default(),"type":"设备","source":"lan-arp-cache","rates":null}));
        }
    }
    result
}
#[derive(Default)]
pub struct LanCache {
    old: BTreeMap<String, (String, f64, u64, u64)>,
    ports: crate::lan_ports::PortCache,
}
impl LanCache {
    pub fn sample(&mut self, bus: &dyn Bus, now: f64) -> Value {
        let began = Instant::now();
        self.sample_with_clock(bus, || now + began.elapsed().as_secs_f64())
    }
    pub fn sample_with_clock(&mut self, bus: &dyn Bus, mut clock: impl FnMut() -> f64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(4);
        let call = |object: &str, method: &str, params: Value| {
            let left = deadline.checked_duration_since(Instant::now())?;
            bus.call(object, method, params, left.min(Duration::from_secs(1)))
                .ok()
        };
        let lan = call("network.interface.lan", "status", json!({}));
        let lan_devices = lan
            .as_ref()
            .filter(|v| v["up"] == true)
            .and_then(|v| v["l3_device"].as_str())
            .filter(|n| name(n))
            .map(|n| vec![n.to_owned()])
            .unwrap_or_default();
        let arp = bounded_read(Path::new("/proc/net/arp"), 65536).unwrap_or_default();
        let leases = bounded_read(Path::new("/tmp/dhcp.leases"), 65536).unwrap_or_default();
        let mut devices = peers(&arp, &leases, &lan_devices);
        let mut status = call("network.wireless", "status", json!({}));
        let mut prefetched = BTreeMap::new();
        if status.as_ref().and_then(Value::as_object).is_none_or(|m| {
            m.values()
                .all(|r| r["interfaces"].as_array().is_none_or(Vec::is_empty))
        }) {
            let (fallback, infos) = fallback_wireless(call);
            prefetched = infos;
            if fallback.as_object().is_some_and(|m| !m.is_empty()) {
                status = Some(fallback);
            }
        }
        let mut wireless = vec![];
        let mut wireless_names: std::collections::BTreeSet<_> =
            prefetched.keys().cloned().collect();
        for radio in status
            .as_ref()
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|v| v.values())
            .take(4)
        {
            for iface in radio["interfaces"]
                .as_array()
                .into_iter()
                .flatten()
                .take(128)
            {
                if let Some(n) = iface["ifname"].as_str().filter(|n| name(n)) {
                    wireless_names.insert(n.to_owned());
                }
            }
        }
        let mut fresh = BTreeMap::new();
        let radios = status.as_ref().and_then(Value::as_object);
        let mut assoc_complete = radios.is_some_and(|r| r.len() <= 4);
        let mut assoc_supported = false;
        for radio in status
            .as_ref()
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|v| v.values())
            .take(4)
        {
            if radio["interfaces"].as_array().is_some_and(|v| v.len() > 4) {
                assoc_complete = false;
            }
            for iface in radio["interfaces"].as_array().into_iter().flatten().take(4) {
                let cfg = &iface["config"];
                let is_lan = cfg["network"] == "lan"
                    || cfg["network"]
                        .as_array()
                        .is_some_and(|v| v.iter().any(|n| n == "lan"));
                let fallback = radio["source"] == "iwinfo-fallback";
                if cfg["mode"] != "ap" || !is_lan && !fallback {
                    continue;
                }
                let Some(ifname) = iface["ifname"].as_str().filter(|n| name(n)) else {
                    continue;
                };
                let state = ap_state(radio, cfg);
                let enabled = state == Some(true);
                let info = if enabled {
                    prefetched
                        .remove(ifname)
                        .or_else(|| call("iwinfo", "info", json!({"device":ifname})))
                        .unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                let freq = info["frequency"].as_u64();
                let band = match freq {
                    Some(2400..=2500) => "2g",
                    Some(4900..=5900) => "5g",
                    Some(5925..=7125) => "6g",
                    _ => "unknown",
                };
                wireless.push(json!({"id":iface["section"],"interface":ifname,"band":band,"ssid":cfg["ssid"].as_str().unwrap_or("").chars().take(32).collect::<String>(),"enabled":state,"stateKnown":state.is_some(),"channel":info["channel"],"channelWidth":info["htmode"],"editable":false,"source":if fallback{"iwinfo-fallback"}else{"netifd-status"},"lanVerified":is_lan}));
                if !enabled || !is_lan {
                    if fallback && !is_lan {
                        assoc_complete = false;
                    }
                    continue;
                }
                let stations = call("iwinfo", "assoclist", json!({"device":ifname}));
                let observed = clock();
                let rows = stations.as_ref().and_then(|v| v["results"].as_array());
                if let Some(rows) = rows {
                    assoc_supported = true;
                    if rows.len() > 128 {
                        assoc_complete = false;
                    }
                } else {
                    assoc_complete = false;
                }
                for station in rows.into_iter().flatten().take(128) {
                    let Some(mac) = station["mac"].as_str().and_then(mac) else {
                        continue;
                    };
                    let device = devices
                        .entry(mac.clone())
                        .or_insert_with(|| json!({"mac":mac,"name":"","ip":null,"rates":null}));
                    device["type"] = json!("WiFi");
                    device["source"] = json!("iwinfo-assoclist");
                    if let (Some(rx), Some(tx)) = (
                        station["rx"]["bytes"].as_u64(),
                        station["tx"]["bytes"].as_u64(),
                    ) {
                        if let Some((old_iface, at, old_rx, old_tx)) =
                            self.old.get(&mac).filter(|old| old.0 == ifname)
                        {
                            let _ = old_iface;
                            device["rates"] = crate::telemetry::station_rates(
                                (*at, *old_rx, *old_tx),
                                (observed, rx, tx),
                            )
                            .unwrap_or(Value::Null);
                        }
                        fresh.insert(mac, (ifname.into(), observed, rx, tx));
                    }
                }
            }
        }
        if devices.len() > 256 {
            assoc_complete = false;
        }
        self.old = fresh;
        let port_samples = if radios.is_some_and(|r| {
            r.len() <= 4
                && r.values().all(|radio| {
                    radio["interfaces"]
                        .as_array()
                        .is_none_or(|v| v.len() <= 128)
                })
        }) {
            lan_devices
                .first()
                .map(|bridge| {
                    crate::lan_ports::collect(Path::new("/sys/class/net"), bridge, &wireless_names)
                })
                .unwrap_or_default()
        } else {
            vec![]
        };
        let ports = self.ports.apply(&mut devices, &port_samples, clock());
        json!({"stale":false,"observedAtMs":crate::events::wall_ms(),"wifiReadSupported":radios.is_some(),"deviceListSupported":!lan_devices.is_empty()||assoc_supported,"complete":assoc_complete,"wireless":wireless,"ports":ports,"devices":devices.into_values().take(256).collect::<Vec<_>>(),"controls":{"editableBands":[],"bandUnifySupported":false}})
    }
}
