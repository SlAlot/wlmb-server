//! 模组配置、总线身份和候选端点清单；使用前核验真实身份与占用。
use crate::{PanelError, Result, catalog};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

fn invalid() -> PanelError {
    PanelError::new("INVALID_MODEM_CONFIG", 422)
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
pub fn valid_port(value: &str) -> bool {
    if value.len() > 160 {
        return false;
    }
    let Some(name) = value.strip_prefix("/dev/") else {
        return false;
    };
    let digits = |v: &str| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit());
    if ["ttyUSB", "ttyACM", "ttyS"]
        .iter()
        .any(|prefix| name.strip_prefix(prefix).is_some_and(digits))
    {
        return true;
    }
    if let Some(rest) = name.strip_prefix("mhi_") {
        return identifier(rest);
    }
    name.strip_prefix("wwan")
        .and_then(|v| v.split_once("at"))
        .is_some_and(|(head, tail)| digits(head) && digits(tail))
}
pub fn at_candidate(row: &Value) -> bool {
    let port = row["port"].as_str().unwrap_or("");
    if !valid_port(port) || port.starts_with("/dev/ttyS") {
        return false;
    }
    let label = row["interfaceHint"].as_str().unwrap_or("");
    let words: Vec<_> = label
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    if words.iter().any(|word| {
        ["diag", "diagnostic", "gps", "nmea", "qdss", "trace"]
            .iter()
            .any(|other| word.eq_ignore_ascii_case(other))
    }) {
        return false;
    }
    port.starts_with("/dev/wwan") || words.iter().any(|word| word.eq_ignore_ascii_case("AT"))
}
fn scalar(raw: &str) -> Option<String> {
    let mut out = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for c in raw.trim().chars() {
        if c.is_control() {
            return None;
        }
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
    (!quoted && !escaped && out.len() <= 160).then_some(out)
}
pub fn parse_owned_config(raw: &str) -> Result<Vec<Value>> {
    if raw.len() > 65536 {
        return Err(invalid());
    }
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            return Err(invalid());
        };
        let Some(key) = key.strip_prefix("network_panel.") else {
            return Err(invalid());
        };
        let (section, key) = key.split_once('.').unwrap_or((key, ".type"));
        if !identifier(section) || key != ".type" && !identifier(key) {
            return Err(invalid());
        }
        if !sections.contains_key(section) && sections.len() >= 64 {
            return Err(invalid());
        }
        let value = scalar(value).ok_or_else(invalid)?;
        if sections
            .entry(section.to_owned())
            .or_default()
            .insert(key.to_owned(), value)
            .is_some()
        {
            return Err(invalid());
        }
    }
    let mut rows = vec![];
    let mut ports = BTreeSet::new();
    for (id, section) in sections {
        if section.get(".type").map(String::as_str) != Some("modem") {
            continue;
        }
        if rows.len() >= 16 || id.len() > 122 {
            return Err(invalid());
        }
        let port = section.get("at_port").ok_or_else(invalid)?;
        let model = section.get("model").ok_or_else(invalid)?;
        if !valid_port(port) || !identifier(model) || !ports.insert(port.clone()) {
            return Err(invalid());
        }
        if section.get("transport").map(String::as_str) != Some("dedicated") {
            return Err(invalid());
        }
        rows.push(json!({"id":format!("panel-{id}"),"port":port,"model":model,
            "transport":"dedicated","shared":false,"channelReady":false,
            "channelState":"dedicated-transport-pending","discoveredBy":"network-panel-config",
            "profile":catalog::identify(model)}));
    }
    Ok(rows)
}
fn metadata(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut raw = Vec::new();
    file.take(257).read_to_end(&mut raw).ok()?;
    if raw.len() > 256 {
        return None;
    }
    let text = std::str::from_utf8(&raw).ok()?.trim();
    (!text.is_empty() && !text.chars().any(char::is_control)).then(|| text.to_owned())
}
pub fn scan_sysfs(root: &Path) -> Result<Vec<Value>> {
    let failure = || PanelError::new("MODEM_DISCOVERY_UNAVAILABLE", 503);
    let root = root.canonicalize().map_err(|_| failure())?;
    let mut rows = vec![];
    let mut seen = BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut visited = 0;
    for class in ["class/tty", "class/wwan"] {
        let entries = match fs::read_dir(root.join(class)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(failure()),
        };
        for entry in entries {
            if visited >= 1024 || Instant::now() >= deadline {
                return Err(failure());
            }
            visited += 1;
            let entry = entry.map_err(|_| failure())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let port = format!("/dev/{name}");
            if !valid_port(&port) || name.starts_with("ttyS") || !seen.insert(port.clone()) {
                continue;
            }
            let device = match entry.path().join("device").canonicalize() {
                Ok(path) if path.starts_with(&root) => path,
                _ => continue,
            };
            let mut interface_hint = None;
            for ancestor in device
                .ancestors()
                .take(16)
                .take_while(|path| path.starts_with(&root))
            {
                if Instant::now() >= deadline {
                    return Err(failure());
                }
                if interface_hint.is_none() {
                    interface_hint = metadata(&ancestor.join("interface"));
                }
                let usb =
                    metadata(&ancestor.join("idVendor")).zip(metadata(&ancestor.join("idProduct")));
                let pci =
                    metadata(&ancestor.join("vendor")).zip(metadata(&ancestor.join("device")));
                let Some((bus, (vendor, product))) = usb
                    .map(|ids| ("usb", ids))
                    .or_else(|| pci.map(|ids| ("pci", ids)))
                else {
                    continue;
                };
                let hex = |value: &str| {
                    let v = value.strip_prefix("0x").unwrap_or(value);
                    v.len() == 4 && v.bytes().all(|b| b.is_ascii_hexdigit())
                };
                if !hex(&vendor) || !hex(&product) {
                    break;
                }
                let physical = ancestor
                    .strip_prefix(&root)
                    .map_err(|_| failure())?
                    .to_string_lossy();
                if physical.len() > 512 || rows.len() >= 64 {
                    return Err(failure());
                }
                rows.push(json!({"port":port,"bus":bus,"physicalDevice":physical,
                "vendorId":vendor,"productId":product,"productHint":metadata(&ancestor.join("product")),
                "interfaceHint":interface_hint,"discoveredBy":"sysfs",
                "identityVerified":false,"channelReady":false}));
                break;
            }
        }
    }
    rows.sort_by(|a, b| a["port"].as_str().cmp(&b["port"].as_str()));
    Ok(rows)
}
