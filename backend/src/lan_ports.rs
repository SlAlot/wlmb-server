//! 网桥转发表与有线端口计数关联；不能唯一归属时不分摊端口速率。
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};

pub type Fdb = BTreeMap<u16, BTreeSet<String>>;
pub fn forwarding(raw: &[u8]) -> Option<Fdb> {
    if raw.len() > 65536 || !raw.chunks_exact(16).remainder().is_empty() {
        return None;
    }
    let mut result = Fdb::new();
    for row in raw.chunks_exact(16) {
        if row[7] != 0 || row[0] & 1 != 0 || row[..6].iter().all(|b| *b == 0) {
            continue;
        }
        let port = u16::from(row[6]) | u16::from(row[12]) << 8;
        if port == 0 {
            continue;
        }
        let address = row[..6]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        result.entry(port).or_default().insert(address);
    }
    Some(result)
}
fn bytes(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let mut raw = vec![];
    std::fs::File::open(path)
        .ok()?
        .take((limit + 1) as u64)
        .read_to_end(&mut raw)
        .ok()?;
    (raw.len() <= limit).then_some(raw)
}
fn number(path: &Path, hex: bool) -> Option<u64> {
    let raw = bytes(path, 128)?;
    let text = std::str::from_utf8(&raw).ok()?.trim();
    if hex {
        u64::from_str_radix(text.strip_prefix("0x").unwrap_or(text), 16).ok()
    } else {
        text.parse().ok()
    }
}
#[derive(Clone)]
pub struct PortSample {
    pub interface: String,
    pub index: u64,
    pub rx: u64,
    pub tx: u64,
    pub peers: BTreeSet<String>,
}
pub fn collect(root: &Path, bridge: &str, wireless: &BTreeSet<String>) -> Vec<PortSample> {
    if !crate::host_controls::interface(bridge) {
        return vec![];
    }
    let base = root.join(bridge);
    let read_fdb = || bytes(&base.join("brforward"), 65536).and_then(|v| forwarding(&v));
    let Some(before) = read_fdb() else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(base.join("brif")) else {
        return vec![];
    };
    let entries: Vec<_> = entries.take(17).collect();
    if entries.len() > 16 {
        return vec![];
    }
    let mut samples = vec![];
    for entry in entries.into_iter().flatten() {
        let Some(interface) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !crate::host_controls::interface(&interface) || wireless.contains(&interface) {
            continue;
        }
        let net = root.join(&interface);
        if !net.join("device").exists()
            || net.join("phy80211").exists()
            || net.join("wireless").exists()
            || number(&net.join("type"), false) != Some(1)
            || number(&net.join("carrier"), false) != Some(1)
            || number(&entry.path().join("state"), false) != Some(3)
        {
            continue;
        }
        let Some(port) =
            number(&entry.path().join("port_no"), true).and_then(|v| u16::try_from(v).ok())
        else {
            continue;
        };
        let Some(peers) = before.get(&port).filter(|p| !p.is_empty()) else {
            continue;
        };
        let Some(index) = number(&net.join("ifindex"), false).filter(|v| *v > 0) else {
            continue;
        };
        let (Some(rx), Some(tx)) = (
            number(&net.join("statistics/rx_bytes"), false),
            number(&net.join("statistics/tx_bytes"), false),
        ) else {
            continue;
        };
        if number(&net.join("ifindex"), false) != Some(index)
            || number(&entry.path().join("port_no"), true) != Some(u64::from(port))
        {
            continue;
        }
        samples.push(PortSample {
            interface,
            index,
            rx,
            tx,
            peers: peers.clone(),
        });
    }
    if read_fdb().as_ref() != Some(&before) {
        return vec![];
    }
    samples
}
#[derive(Default)]
pub struct PortCache {
    old: BTreeMap<String, (u64, f64, u64, u64, Option<String>)>,
}
impl PortCache {
    pub fn apply(
        &mut self,
        devices: &mut BTreeMap<String, Value>,
        samples: &[PortSample],
        now: f64,
    ) -> Vec<Value> {
        let mut fresh = BTreeMap::new();
        let mut rows = vec![];
        for device in devices.values_mut() {
            if device["rateSource"] == "ethernet-port-counter-delta" {
                if device["type"] != "WiFi" {
                    device["rates"] = Value::Null;
                }
                if device["type"] == "有线" {
                    device["type"] = json!("设备");
                }
                if let Some(fields) = device.as_object_mut() {
                    for key in ["rateScope", "rateInterface", "rateSource"] {
                        fields.remove(key);
                    }
                }
            }
        }
        for sample in samples.iter().take(16) {
            let sole = (sample.peers.len() == 1)
                .then(|| sample.peers.iter().next().cloned())
                .flatten();
            let old = self
                .old
                .get(&sample.interface)
                .filter(|p| p.0 == sample.index);
            let rates = old.and_then(|p| {
                crate::telemetry::station_rates((p.1, p.2, p.3), (now, sample.rx, sample.tx))
            });
            if let Some(address) = sole
                .as_ref()
                .filter(|m| samples.iter().filter(|s| s.peers.contains(*m)).count() == 1)
            {
                if let Some(device) = devices.get_mut(address).filter(|d| d["type"] != "WiFi") {
                    device["type"] = json!("有线");
                    device["rateScope"] = json!("port");
                    device["rateInterface"] = json!(sample.interface);
                    device["rateSource"] = json!("ethernet-port-counter-delta");
                    device["rates"] = old
                        .filter(|p| p.4.as_ref() == Some(address))
                        .and_then(|_| rates.clone())
                        .unwrap_or(Value::Null);
                }
            }
            rows.push(json!({"interface":sample.interface,"source":"ethernet-port-counter-delta","scope":"port","peerCount":sample.peers.len(),"rates":rates}));
            fresh.insert(
                sample.interface.clone(),
                (sample.index, now, sample.rx, sample.tx, sole),
            );
        }
        self.old = fresh;
        rows
    }
}
