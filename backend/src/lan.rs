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
        let call = |object, method, params| {
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
        let status = call("network.wireless", "status", json!({}));
        let mut wireless = vec![];
        let mut wireless_names = std::collections::BTreeSet::new();
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
                if cfg["mode"] != "ap" || !is_lan {
                    continue;
                }
                let Some(ifname) = iface["ifname"].as_str().filter(|n| name(n)) else {
                    continue;
                };
                let state = ap_state(radio, cfg);
                let enabled = state == Some(true);
                let info = if enabled {
                    call("iwinfo", "info", json!({"device":ifname})).unwrap_or(Value::Null)
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
                wireless.push(json!({"id":iface["section"],"interface":ifname,"band":band,"ssid":cfg["ssid"].as_str().unwrap_or("").chars().take(32).collect::<String>(),"enabled":state,"stateKnown":state.is_some(),"channel":info["channel"],"channelWidth":info["htmode"],"editable":false}));
                if !enabled {
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
