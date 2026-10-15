#![cfg(unix)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
#![allow(clippy::unwrap_used)]
use nix::fcntl::{FlockArg, flock};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::{
        fs::{MetadataExt, PermissionsExt, symlink},
        io::AsRawFd,
    },
    path::PathBuf,
    process::Command,
};
use wlmb_server::qmodem_queue::Contract;
struct Fixture {
    root: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "wlmb-queue-{}-{:x}",
            std::process::id(),
            u128::from_le_bytes(random)
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let source = root.join("reviewed-source");
        fs::write(&source, b"reviewed queue ABI").unwrap();
        Self { root, source }
    }
    fn contract(&self) -> Contract {
        Contract::verify(
            &self.root,
            &[(
                &self.source,
                &format!("{:x}", Sha256::digest(b"reviewed queue ABI")),
            )],
            fs::metadata(&self.root).unwrap().uid(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn own_fd_lock_excludes_other_process_and_preserves_pid_file() {
    let f = Fixture::new();
    let path = f.root.join("qmodem-at-ttyUSB1.lock");
    fs::write(&path, b"prior-keeper\n").unwrap();
    let q = f.contract();
    let held = q.acquire("/dev/ttyUSB1").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"prior-keeper\n");
    assert!(
        !Command::new("flock")
            .args(["-n", path.to_str().unwrap(), "true"])
            .status()
            .unwrap()
            .success()
    );
    assert!(q.acquire("/dev/ttyUSB1").is_err());
    drop(held);
    assert!(
        Command::new("flock")
            .args(["-n", path.to_str().unwrap(), "true"])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(&path).unwrap(), b"prior-keeper\n");
}
#[test]
fn global_contention_releases_partial_port_lock_without_wait_or_unlink() {
    let f = Fixture::new();
    let q = f.contract();
    let global = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(f.root.join("qmodem-at-daemon.lock"))
        .unwrap();
    flock(global.as_raw_fd(), FlockArg::LockExclusiveNonblock).unwrap();
    assert_eq!(q.acquire("/dev/ttyUSB1").err().unwrap().code, "AT_BUSY");
    let port = fs::File::open(f.root.join("qmodem-at-ttyUSB1.lock")).unwrap();
    flock(port.as_raw_fd(), FlockArg::LockExclusiveNonblock).unwrap();
    drop(port);
    drop(global);
    assert!(q.acquire("/dev/ttyUSB1").is_ok());
}
#[test]
fn source_change_inode_replacement_and_insecure_files_fail_closed() {
    let f = Fixture::new();
    let q = f.contract();
    let held = q.acquire("/dev/ttyUSB1").unwrap();
    let port = f.root.join("qmodem-at-ttyUSB1.lock");
    fs::rename(&port, f.root.join("old-lock")).unwrap();
    fs::write(&port, b"").unwrap();
    assert!(held.validate().is_err());
    drop(held);
    fs::write(&f.source, b"changed ABI").unwrap();
    assert!(q.validate().is_err());
    assert!(q.acquire("/dev/ttyUSB1").is_err());
    let f = Fixture::new();
    let q = f.contract();
    let port = f.root.join("qmodem-at-ttyUSB1.lock");
    symlink(&f.source, &port).unwrap();
    assert!(q.acquire("/dev/ttyUSB1").is_err());
    let f = Fixture::new();
    let q = f.contract();
    let port = f.root.join("qmodem-at-ttyUSB1.lock");
    fs::hard_link(&f.source, &port).unwrap();
    assert!(q.acquire("/dev/ttyUSB1").is_err());
    let f = Fixture::new();
    let q = f.contract();
    fs::set_permissions(&f.root, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(q.validate().is_err());
}
#[test]
fn unknown_abi_and_non_device_paths_do_not_authorize_shared_writes() {
    let f = Fixture::new();
    assert!(
        Contract::verify(
            &f.root,
            &[(&f.source, "wrong")],
            fs::metadata(&f.root).unwrap().uid()
        )
        .is_err()
    );
    assert!(Contract::verify(&f.root, &[], 0).is_err());
    let q = f.contract();
    for port in ["/dev/../tmp/foo", "/tmp/ttyUSB1", "/dev/ttyUSB1;reboot", ""] {
        assert!(q.acquire(port).is_err());
    }
}
#[test]
fn openwrt_sticky_lock_directory_is_supported_but_world_writable_files_are_not() {
    let f = Fixture::new();
    fs::set_permissions(&f.root, fs::Permissions::from_mode(0o1777)).unwrap();
    let q = f.contract();
    let held = q.acquire("/dev/ttyUSB1").unwrap();
    drop(held);
    fs::set_permissions(
        f.root.join("qmodem-at-ttyUSB1.lock"),
        fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    assert!(q.acquire("/dev/ttyUSB1").is_err());
}
