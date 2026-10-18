//! 同机只读文件与原 modem 事务工具封装；不使用管理地址，不打开模组串口。
use super::{Input, Io, ToolReply};
use crate::{PanelError, Result, luoye_info, transport};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub struct SystemIo {
    neighbors: bool,
}
fn failed() -> PanelError {
    PanelError::new("MODEM_CACHE_UNAVAILABLE", 503)
}
fn trusted(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.uid() == 0 && m.mode() & 0o022 == 0)
}
impl SystemIo {
    pub fn detected(root: &Path) -> bool {
        [
            "usr/libexec/modem/at",
            "usr/share/modem/modem_info_cache.sh",
        ]
        .iter()
        .all(|name| trusted(&root.join(name)))
    }
    pub fn new() -> Self {
        let neighbors = match transport::process(
            "/sbin/uci",
            &["-q", "get", "network_panel.main.luoye_neighbors"],
            None,
            Duration::from_secs(2),
        ) {
            Ok(value) => value.trim() == "1",
            Err(_) => true,
        };
        Self { neighbors }
    }
}
impl Default for SystemIo {
    fn default() -> Self {
        Self::new()
    }
}
impl Io for SystemIo {
    fn neighbors_enabled(&self) -> bool {
        self.neighbors
    }
    fn load(&self) -> Result<Input> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open("/tmp/modem-info-_dev_modem_at.json")
            .map_err(|_| failed())?;
        let meta = file.metadata().map_err(|_| failed())?;
        if !meta.is_file()
            || meta.uid() != 0
            || meta.mode() & 0o022 != 0
            || meta.len() > luoye_info::MAX_CACHE as u64
        {
            return Err(failed());
        }
        let mut bytes = vec![];
        (&file)
            .take(luoye_info::MAX_CACHE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failed())?;
        let after = file.metadata().map_err(|_| failed())?;
        if bytes.len() > luoye_info::MAX_CACHE
            || (
                meta.len(),
                meta.mtime(),
                meta.mtime_nsec(),
                meta.ctime(),
                meta.ctime_nsec(),
            ) != (
                after.len(),
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec(),
            )
        {
            return Err(failed());
        }
        let raw = String::from_utf8(bytes).map_err(|_| failed())?;
        let published_at_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|v| v.as_millis().try_into().ok());
        let boot = transport::bounded_read(Path::new("/proc/sys/kernel/random/boot_id"), 64)?;
        let boot = boot.trim();
        if boot.len() != 36 || !boot.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return Err(failed());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| failed())?
            .as_secs_f64();
        let uptime = transport::bounded_read(Path::new("/proc/uptime"), 128)
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .filter(|v| v.is_finite() && *v >= 0.);
        let device = fs::metadata("/dev/modem-at")
            .ok()
            .filter(|m| m.file_type().is_char_device());
        let link = fs::symlink_metadata("/dev/modem-at").ok();
        let identity_valid = device.as_ref().zip(link.as_ref()).zip(uptime).is_some_and(
            |((device, link), uptime)| {
                published_at_ms.is_some_and(|at| {
                    at as f64 / 1000. >= now - uptime
                        && meta.mtime() >= device.ctime()
                        && meta.mtime() >= link.ctime()
                })
            },
        );
        let instance = device
            .as_ref()
            .zip(link.as_ref())
            .map(|(d, l)| {
                format!(
                    "{boot}:{}:{}:{}:{}:{}:{}:{}",
                    d.dev(),
                    d.ino(),
                    d.rdev(),
                    d.ctime(),
                    d.ctime_nsec(),
                    l.ino(),
                    l.ctime()
                )
            })
            .unwrap_or_else(|| format!("{boot}:unbound"));
        Ok(Input {
            raw,
            published_at_ms,
            instance,
            identity_valid,
        })
    }
    fn neighbor(&self, request: &serde_json::Value) -> Result<ToolReply> {
        if !self.neighbors
            || !super::request_valid(request)
            || !trusted(Path::new("/usr/libexec/modem/at"))
        {
            return Err(PanelError::new("AT_WRITE_DISABLED", 403));
        }
        let bytes = serde_json::to_vec(request).map_err(|_| failed())?;
        let (success, raw) = transport::process_capture(
            "/usr/libexec/modem/at",
            &["--json"],
            Some(&bytes),
            Duration::from_secs(12),
        )
        .map_err(|_| PanelError::new("MODEM_NEIGHBOR_RESULT_UNKNOWN", 503))?;
        let value =
            serde_json::from_str(&raw).map_err(|_| PanelError::new("AT_RESPONSE_MISMATCH", 503))?;
        Ok(ToolReply { success, value })
    }
}
