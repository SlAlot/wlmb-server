//! 私有状态的权限核验与原子持久化；拒绝符号链接和不可信目录。
use crate::{PanelError, Result};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
pub struct PrivateStore {
    directory: PathBuf,
}
fn bad() -> PanelError {
    PanelError::new("PRIVATE_STORE_FAILED", 503)
}
impl PrivateStore {
    pub fn open(directory: &Path) -> Result<Self> {
        crate::sms_forward::DiskJournal::open(directory)?;
        Ok(Self {
            directory: directory.into(),
        })
    }
    fn path(&self, name: &str) -> Result<PathBuf> {
        if ![
            "traffic.json",
            "settings.json",
            "host.json",
            "wifi-backup.json",
            "sms-read.json",
        ]
        .contains(&name)
        {
            return Err(bad());
        }
        Self::open(&self.directory)?;
        Ok(self.directory.join(name))
    }
    pub fn load(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = self.path(name)?;
        let before = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(bad()),
        };
        if !before.is_file()
            || before.uid() != 0
            || before.mode() & 0o077 != 0
            || before.nlink() != 1
            || before.len() > 16384
        {
            return Err(bad());
        }
        let f = fs::File::open(path).map_err(|_| bad())?;
        let after = f.metadata().map_err(|_| bad())?;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(bad());
        }
        let mut raw = vec![];
        f.take(16385).read_to_end(&mut raw).map_err(|_| bad())?;
        if raw.len() > 16384 {
            return Err(bad());
        }
        Ok(Some(raw))
    }
    pub fn save(&self, name: &str, raw: &[u8]) -> Result<()> {
        let path = self.path(name)?;
        if raw.len() > 16384 {
            return Err(bad());
        }
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| bad())?;
        let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let tmp = self.directory.join(format!(".{name}-{suffix}.tmp"));
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|_| bad())?;
        let result = (|| {
            f.write_all(raw).map_err(|_| bad())?;
            f.sync_all().map_err(|_| bad())?;
            fs::rename(&tmp, path).map_err(|_| bad())?;
            fs::File::open(&self.directory)
                .and_then(|d| d.sync_all())
                .map_err(|_| bad())
        })();
        drop(f);
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
}
