//! 通用电池快照；标准节点自动发现，特殊设备由本机生产者提供有时效的归一化数据。
#[cfg(any(unix, test))]
use serde::Deserialize;
use serde_json::{Value, json};

pub const PROVIDER_PATH: &str = "/var/run/wlmb-server/battery.json";
pub const PROVIDER_MAX_AGE_SECONDS: u64 = 60;

#[cfg(any(unix, test))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    version: u8,
    percent: Option<u8>,
    charging: Option<bool>,
    #[serde(rename = "updatedAt")]
    updated_at: u64,
}

#[cfg(any(unix, test))]
fn missing(supported: bool, source: &str, reason: &str) -> Value {
    json!({"supported":supported,"available":false,"percent":null,"charging":null,
        "source":source,"observedAtMs":null,"stale":true,"reason":reason})
}
#[cfg(any(unix, test))]
fn reading(percent: u8, charging: Option<bool>, source: &str, at: u64) -> Value {
    json!({"supported":true,"available":true,"percent":percent,"charging":charging,
        "source":source,"observedAtMs":at,"stale":false,"reason":null})
}
#[cfg(any(unix, test))]
fn report(raw: &[u8], now_ms: u64) -> Value {
    let bad = || missing(true, "provider", "BATTERY_REPORT_INVALID");
    if raw.is_empty() || raw.len() > 1024 {
        return bad();
    }
    let Ok(value) = serde_json::from_slice::<Report>(raw) else {
        return bad();
    };
    if value.version != 1 || value.percent.is_some_and(|p| p > 100) {
        return bad();
    }
    let Some(at) = value.updated_at.checked_mul(1000) else {
        return bad();
    };
    if at == 0
        || at > now_ms.saturating_add(5000)
        || now_ms.saturating_sub(at) > PROVIDER_MAX_AGE_SECONDS * 1000
    {
        return missing(true, "provider", "BATTERY_REPORT_EXPIRED");
    }
    let Some(percent) = value.percent else {
        return missing(true, "provider", "BATTERY_VALUE_UNAVAILABLE");
    };
    reading(percent, value.charging, "provider", at)
}

/// 只改变有效性，不延长生产者时间；过期电量不能继续被当作当前电量展示。
pub fn age(value: &mut Value, now_ms: u64, system_stale: bool) {
    let limit = if value["source"] == "provider" {
        PROVIDER_MAX_AGE_SECONDS * 1000
    } else {
        15000
    };
    let expired = value["observedAtMs"]
        .as_u64()
        .is_none_or(|at| at > now_ms.saturating_add(5000) || now_ms.saturating_sub(at) > limit);
    if value["stale"] == true || system_stale || expired {
        value["available"] = json!(false);
        value["stale"] = json!(true);
        value["percent"] = Value::Null;
        value["charging"] = Value::Null;
        if value["reason"].is_null() {
            value["reason"] = json!("BATTERY_SOURCE_STALE");
        }
    }
}

#[cfg(unix)]
mod system {
    use super::*;
    use std::{
        fs,
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::Path,
    };

    // 固定路径只读；不执行外部命令，不跟随生产者文件链接，不等待管道。
    fn provider(path: &Path, at: u64, owner: u32) -> Option<Value> {
        let invalid = || missing(true, "provider", "BATTERY_REPORT_UNTRUSTED");
        let before = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => return Some(invalid()),
        };
        let safe = || -> std::io::Result<Vec<u8>> {
            let directory = fs::symlink_metadata(
                path.parent()
                    .ok_or_else(|| std::io::Error::other("parent"))?,
            )?;
            if !directory.is_dir()
                || directory.uid() != owner
                || directory.mode() & 0o022 != 0
                || !before.is_file()
                || before.uid() != owner
                || before.mode() & 0o022 != 0
                || before.nlink() != 1
                || before.len() > 1024
            {
                return Err(std::io::Error::other("metadata"));
            }
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
                .open(path)?;
            let after = file.metadata()?;
            if before.dev() != after.dev() || before.ino() != after.ino() {
                return Err(std::io::Error::other("changed"));
            }
            let mut bytes = vec![];
            file.take(1025).read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        Some(match safe() {
            Ok(bytes) => report(&bytes, at),
            Err(_) => invalid(),
        })
    }
    fn text(path: &Path) -> Option<String> {
        let mut data = String::new();
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NONBLOCK)
            .open(path)
            .ok()?
            .take(65)
            .read_to_string(&mut data)
            .ok()?;
        (data.len() <= 64).then_some(data)
    }
    fn supplies(root: &Path, at: u64) -> Value {
        let Ok(entries) = fs::read_dir(root) else {
            return missing(false, "power-supply", "BATTERY_NOT_DETECTED");
        };
        let mut candidates = vec![];
        for (count, entry) in entries.enumerate() {
            if count >= 32 {
                return missing(false, "power-supply", "BATTERY_DISCOVERY_LIMIT");
            }
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            let Some(name) = name.to_str().filter(|s| {
                s.len() <= 64
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
            }) else {
                continue;
            };
            let path = root.join(name);
            if text(&path.join("type")).as_deref().map(str::trim) != Some("Battery") {
                continue;
            }
            match text(&path.join("present")).as_deref().map(str::trim) {
                Some("0") => continue,
                Some("1") | None => {}
                _ => return missing(true, "power-supply", "BATTERY_VALUE_UNAVAILABLE"),
            }
            candidates.push(path);
        }
        if candidates.len() > 1 {
            return missing(true, "power-supply", "BATTERY_AMBIGUOUS");
        }
        let Some(path) = candidates.first() else {
            return missing(false, "power-supply", "BATTERY_NOT_DETECTED");
        };
        let percent = text(&path.join("capacity"))
            .and_then(|s| s.trim().parse::<u8>().ok())
            .filter(|p| *p <= 100);
        let Some(percent) = percent else {
            return missing(true, "power-supply", "BATTERY_VALUE_UNAVAILABLE");
        };
        let charging = match text(&path.join("status")).as_deref().map(str::trim) {
            Some("Charging") => Some(true),
            Some("Discharging" | "Full" | "Not charging") => Some(false),
            _ => None,
        };
        reading(percent, charging, "power-supply", at)
    }
    pub(super) fn collect(root: &Path, path: &Path, at: u64, owner: u32) -> Value {
        provider(path, at, owner).unwrap_or_else(|| supplies(root, at))
    }
    pub fn snapshot(at: u64) -> Value {
        collect(
            Path::new("/sys/class/power_supply"),
            Path::new(PROVIDER_PATH),
            at,
            0,
        )
    }

    #[cfg(test)]
    #[allow(clippy::unwrap_used)]
    mod tests {
        use super::*;
        use std::{
            os::unix::fs::{PermissionsExt, symlink},
            path::PathBuf,
            sync::atomic::{AtomicU64, Ordering},
        };
        struct Fixture {
            directory: PathBuf,
            owner: u32,
        }
        impl Fixture {
            fn new() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let directory = std::env::temp_dir().join(format!(
                    "wlmb-battery-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                fs::create_dir(&directory).unwrap();
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
                let owner = fs::metadata(&directory).unwrap().uid();
                Self { directory, owner }
            }
            fn supply(&self, name: &str, kind: &str, capacity: &str, status: &str) {
                let p = self.directory.join(name);
                fs::create_dir(&p).unwrap();
                for (key, value) in [("type", kind), ("capacity", capacity), ("status", status)] {
                    fs::write(p.join(key), value).unwrap();
                }
            }
            fn sample(&self) -> Value {
                collect(
                    &self.directory,
                    &self.directory.join("battery.json"),
                    100000,
                    self.owner,
                )
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.directory);
            }
        }
        #[test]
        fn scan_ignores_chargers_and_preserves_real_zero() {
            let f = Fixture::new();
            f.supply("usb", "USB", "100", "Charging");
            assert_eq!(f.sample()["supported"], false);
            f.supply("pack0", "Battery", "0\n", "Discharging");
            assert_eq!(f.sample()["percent"], 0);
            assert_eq!(f.sample()["charging"], false);
            fs::write(f.directory.join("pack0/present"), "0").unwrap();
            assert_eq!(f.sample()["supported"], false);
        }
        #[test]
        fn ambiguous_or_invalid_power_never_becomes_a_fake_percentage() {
            let f = Fixture::new();
            f.supply("pack0", "Battery", "255", "Unknown");
            assert_eq!(f.sample()["available"], false);
            fs::write(f.directory.join("pack0/capacity"), "42").unwrap();
            assert_eq!(f.sample()["charging"], Value::Null);
            f.supply("pack1", "Battery", "99", "Full");
            assert_eq!(f.sample()["reason"], "BATTERY_AMBIGUOUS");
        }
        #[test]
        fn provider_priority_expiry_and_permissions_are_explicit() {
            let f = Fixture::new();
            f.supply("pack0", "Battery", "50", "Charging");
            let p = f.directory.join("battery.json");
            fs::write(
                &p,
                br#"{"version":1,"percent":70,"charging":null,"updatedAt":100}"#,
            )
            .unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(f.sample()["percent"], 70);
            assert_eq!(f.sample()["source"], "provider");
            assert_eq!(
                collect(&f.directory, &p, 161000, f.owner)["reason"],
                "BATTERY_REPORT_EXPIRED"
            );
            fs::set_permissions(&p, fs::Permissions::from_mode(0o666)).unwrap();
            assert_eq!(f.sample()["reason"], "BATTERY_REPORT_UNTRUSTED");
        }
        #[test]
        fn provider_links_and_oversized_files_are_rejected() {
            let f = Fixture::new();
            let p = f.directory.join("battery.json");
            let other = f.directory.join("other");
            fs::write(&other, "{}").unwrap();
            symlink(&other, &p).unwrap();
            assert_eq!(f.sample()["available"], false);
            fs::remove_file(&p).unwrap();
            fs::write(&p, vec![b' '; 1025]).unwrap();
            assert_eq!(f.sample()["available"], false);
        }
        #[test]
        fn provider_pipe_hardlink_and_wrong_owner_do_not_become_inputs() {
            let f = Fixture::new();
            let p = f.directory.join("battery.json");
            nix::unistd::mkfifo(&p, nix::sys::stat::Mode::S_IRUSR).unwrap();
            assert_eq!(f.sample()["reason"], "BATTERY_REPORT_UNTRUSTED");
            fs::remove_file(&p).unwrap();
            fs::write(&p, br#"{"version":1,"percent":80,"updatedAt":100}"#).unwrap();
            assert_eq!(
                collect(&f.directory, &p, 100000, f.owner.wrapping_add(1))["reason"],
                "BATTERY_REPORT_UNTRUSTED"
            );
            fs::hard_link(&p, f.directory.join("copy")).unwrap();
            assert_eq!(f.sample()["reason"], "BATTERY_REPORT_UNTRUSTED");
        }
    }
}
#[cfg(unix)]
pub use system::snapshot;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_require_typed_bounded_fields_and_reject_duplicate_keys() {
        let good = br#"{"version":1,"percent":100,"charging":false,"updatedAt":100}"#;
        assert_eq!(report(good, 100000)["percent"], 100);
        for raw in [
            r#"{"version":1,"percent":101,"updatedAt":100}"#,
            r#"{"version":1,"percent":"80","updatedAt":100}"#,
            r#"{"version":1,"percent":80,"charging":1,"updatedAt":100}"#,
            r#"{"version":1,"percent":80,"percent":90,"updatedAt":100}"#,
            r#"{"version":1,"percent":80,"updatedAt":100,"command":"unused"}"#,
            r#"{"version":2,"percent":80,"updatedAt":100}"#,
        ] {
            assert_eq!(report(raw.as_bytes(), 100000)["available"], false);
        }
    }
    #[test]
    fn expired_future_and_unavailable_reports_do_not_fall_back_to_zero() {
        for time in [0, 1, 106, u64::MAX] {
            let v = report(
                format!(r#"{{"version":1,"percent":40,"updatedAt":{time}}}"#).as_bytes(),
                100000,
            );
            assert_eq!(v["available"], false);
            assert!(v["percent"].is_null());
        }
        assert_eq!(
            report(br#"{"version":1,"percent":null,"updatedAt":100}"#, 100000)["reason"],
            "BATTERY_VALUE_UNAVAILABLE"
        );
    }
    #[test]
    fn response_age_and_system_failure_invalidate_cached_percent() {
        let mut v = reading(50, Some(true), "provider", 100000);
        age(&mut v, 161000, false);
        assert!(v["percent"].is_null());
        let mut v = reading(50, Some(true), "power-supply", 100000);
        age(&mut v, 100001, true);
        assert_eq!(v["available"], false);
        let mut v = reading(0, None, "provider", 100000);
        age(&mut v, 160000, false);
        assert_eq!(v["percent"], 0);
    }
}
