//! 参考QModem公开队列契约串行执行短信事务，超时结果不重放。
use crate::{PanelError, Result};
use nix::fcntl::{FlockArg, flock};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
};
fn changed() -> PanelError {
    PanelError::new("SHARED_QUEUE_CHANGED", 409)
}
#[derive(Clone, PartialEq)]
struct Stamp {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: i64,
    ns: i64,
    uid: u32,
    mode: u32,
}
fn stamp(path: &Path, uid: u32) -> Result<Stamp> {
    let m = fs::symlink_metadata(path).map_err(|_| changed())?;
    if !m.is_file() || m.nlink() != 1 || m.uid() != uid || m.mode() & 0o022 != 0 {
        return Err(changed());
    }
    Ok(Stamp {
        dev: m.dev(),
        ino: m.ino(),
        size: m.len(),
        mtime: m.mtime(),
        ns: m.mtime_nsec(),
        uid: m.uid(),
        mode: m.mode(),
    })
}
pub struct Contract {
    root: PathBuf,
    uid: u32,
    identity: (u64, u64),
    sources: Vec<(PathBuf, Stamp)>,
}
impl Contract {
    pub fn verify(root: &Path, sources: &[(&Path, &str)], uid: u32) -> Result<Self> {
        if sources.is_empty() || sources.len() > 4 {
            return Err(changed());
        }
        let root = fs::canonicalize(root).map_err(|_| changed())?;
        let m = fs::metadata(&root).map_err(|_| changed())?;
        if !m.is_dir() || m.uid() != uid || (m.mode() & 0o022 != 0 && m.mode() & 0o1000 == 0) {
            return Err(changed());
        }
        let mut stamps = vec![];
        for (path, expected) in sources {
            let before = stamp(path, uid)?;
            let mut raw = vec![];
            File::open(path)
                .map_err(|_| changed())?
                .take(262145)
                .read_to_end(&mut raw)
                .map_err(|_| changed())?;
            if raw.len() > 262144
                || format!("{:x}", Sha256::digest(&raw)) != *expected
                || stamp(path, uid)? != before
            {
                return Err(changed());
            }
            stamps.push((path.to_path_buf(), before));
        }
        Ok(Self {
            root,
            uid,
            identity: (m.dev(), m.ino()),
            sources: stamps,
        })
    }
    pub fn installed() -> Option<Self> {
        Self::verify(
            Path::new("/var/lock"),
            &[
                (
                    Path::new("/usr/share/qmodem/modem_util.sh"),
                    "383b873fbb527dcb542c13409c1909f9e820bfbd9d0ae29ea15fe2ed7de6fe84",
                ),
                (
                    Path::new("/usr/libexec/rpcd/qmodem"),
                    "237c3f05551cf9e008e9d50e65b6bab9ea2bd9c76a6132d6a25ebfa4899fedb0",
                ),
            ],
            0,
        )
        .ok()
    }
    pub fn validate(&self) -> Result<()> {
        let m = fs::metadata(&self.root).map_err(|_| changed())?;
        if !m.is_dir()
            || (m.dev(), m.ino()) != self.identity
            || m.uid() != self.uid
            || (m.mode() & 0o022 != 0 && m.mode() & 0o1000 == 0)
        {
            return Err(changed());
        }
        for (path, before) in &self.sources {
            if stamp(path, self.uid)? != *before {
                return Err(changed());
            }
        }
        Ok(())
    }
    pub fn acquire(&self, port: &str) -> Result<Held> {
        self.validate()?;
        if !crate::transport::valid_port(port) {
            return Err(changed());
        }
        let canonical = fs::canonicalize(port).unwrap_or_else(|_| PathBuf::from(port));
        let name = canonical
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(changed)?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        {
            return Err(changed());
        }
        let mut files = vec![];
        for name in [
            format!("qmodem-at-{name}.lock"),
            "qmodem-at-daemon.lock".into(),
        ] {
            let path = self.root.join(name);
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o700)
                .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
                .open(&path)
                .map_err(|_| changed())?;
            let m = file.metadata().map_err(|_| changed())?;
            let expected = stamp(&path, self.uid)?;
            if (m.dev(), m.ino()) != (expected.dev, expected.ino) {
                return Err(changed());
            }
            flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock).map_err(|e| {
                if e == nix::errno::Errno::EAGAIN {
                    PanelError::new("AT_BUSY", 409)
                } else {
                    changed()
                }
            })?;
            files.push((path, file, (m.dev(), m.ino())));
        }
        let held = Held { files };
        held.validate()?;
        self.validate()?;
        Ok(held)
    }
}
pub struct Held {
    files: Vec<(PathBuf, File, (u64, u64))>,
}
impl Held {
    pub fn validate(&self) -> Result<()> {
        for (path, file, identity) in &self.files {
            let current = fs::symlink_metadata(path).map_err(|_| changed())?;
            let held = file.metadata().map_err(|_| changed())?;
            if !current.is_file()
                || current.nlink() != 1
                || (current.dev(), current.ino()) != *identity
                || (held.dev(), held.ino()) != *identity
            {
                return Err(changed());
            }
        }
        Ok(())
    }
}
