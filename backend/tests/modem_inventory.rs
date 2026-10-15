#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use wlmb_server::modem_inventory::{parse_owned_config, scan_sysfs, valid_port};

const OWN: &str = "network_panel.main=core\nnetwork_panel.main.interval='3'\nnetwork_panel.modem1=modem\nnetwork_panel.modem1.at_port='/dev/ttyUSB1'\nnetwork_panel.modem1.model='mt5700m-cn'\nnetwork_panel.modem1.transport='dedicated'\n";
#[test]
fn owned_inventory_is_not_a_shared_daemon_or_qmodem_profile() {
    let rows = parse_owned_config(OWN).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "panel-modem1");
    assert_eq!(rows[0]["profile"]["family"], "tdtech");
    assert_eq!(rows[0]["shared"], false);
    assert_eq!(rows[0]["channelReady"], false);
    assert_eq!(rows[0]["discoveredBy"], "network-panel-config");
    assert!(parse_owned_config(&OWN.replace("network_panel.", "qmodem.")).is_err());
    assert!(
        parse_owned_config("network_panel.main=core\nnetwork_panel.main.interval='3'\n")
            .unwrap()
            .is_empty()
    );
}
#[test]
fn explicit_invalid_configuration_never_falls_back_or_executes_scalars() {
    for (from, to) in [
        ("/dev/ttyUSB1", "/dev/../ttyUSB1"),
        ("/dev/ttyUSB1", "$(touch /tmp/owned)"),
        ("mt5700m-cn", "model;reboot"),
        ("'dedicated'", "'ubus'"),
        ("'dedicated'", "'dedicated"),
        ("modem1", "@modem[0]"),
    ] {
        assert!(parse_owned_config(&OWN.replace(from, to)).is_err(), "{to}");
    }
    assert!(
        parse_owned_config(&OWN.replace("network_panel.modem1.model='mt5700m-cn'\n", "")).is_err()
    );
    assert!(
        parse_owned_config(&(OWN.to_owned() + "network_panel.modem1.model='rm520n-gl'\n")).is_err()
    );
    assert!(
        parse_owned_config(
            &(OWN.to_owned()
                + &OWN.replace("modem1", "modem2").replace(
                    "network_panel.main=core\nnetwork_panel.main.interval='3'\n",
                    ""
                ))
        )
        .is_err()
    );
    assert!(parse_owned_config(&"x".repeat(65537)).is_err());
}
#[test]
fn endpoints_are_literal_and_bounded_across_usb_and_pcie() {
    for good in [
        "/dev/ttyUSB1",
        "/dev/ttyACM10",
        "/dev/ttyS0",
        "/dev/wwan0at0",
        "/dev/mhi_DUN",
    ] {
        assert!(valid_port(good));
    }
    for bad in [
        "/dev/ttyUSB",
        "/dev/wwan0at",
        "/dev/mhi_",
        "/dev/ttyUSB1;reboot",
        "/dev/../ttyUSB1",
        "ttyUSB1",
        "/tmp/ttyUSB1",
    ] {
        assert!(!valid_port(bad));
    }
}
struct Tree(PathBuf);
impl Tree {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "wlmb-inventory-{}-{}-{}",
            std::process::id(),
            wlmb_server::events::wall_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(root.join("class/tty")).unwrap();
        Self(root)
    }
    fn usb(&self, port: &str, vendor: &str) -> PathBuf {
        let device = self.0.join("class/tty").join(port).join("device");
        fs::create_dir_all(&device).unwrap();
        for (name, value) in [
            ("idVendor", vendor),
            ("idProduct", "15c1"),
            ("product", "MT5700M-CN"),
            ("interface", "Modem AT"),
        ] {
            fs::write(device.join(name), value).unwrap();
        }
        device
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn sysfs_discovery_reads_only_metadata_and_does_not_claim_at_readiness() {
    let tree = Tree::new();
    tree.usb("ttyUSB1", "12d1");
    tree.usb("ttyS0", "12d1");
    tree.usb("ttyUSB9", "not-a-vendor");
    fs::create_dir_all(tree.0.join("class/tty/ttyUSB2/device")).unwrap();
    let rows = scan_sysfs(&tree.0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["port"], "/dev/ttyUSB1");
    assert_eq!(rows[0]["productHint"], "MT5700M-CN");
    assert_eq!(rows[0]["identityVerified"], false);
    assert_eq!(rows[0]["channelReady"], false);
    assert_eq!(rows[0]["bus"], "usb");
    assert!(rows[0].get("profile").is_none());
}
#[test]
fn sysfs_bad_or_oversized_hints_do_not_become_commands_or_identity() {
    let tree = Tree::new();
    let node = tree.usb("ttyUSB1", "12d1");
    fs::write(node.join("product"), "x".repeat(257)).unwrap();
    fs::write(node.join("interface"), "AT\u{0}reboot").unwrap();
    let rows = scan_sysfs(&tree.0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["productHint"], json!(null));
    assert_eq!(rows[0]["interfaceHint"], json!(null));
}
#[test]
fn pcie_wwan_at_endpoint_is_metadata_only_and_does_not_require_usb_tty() {
    let tree = Tree::new();
    fs::remove_dir(tree.0.join("class/tty")).unwrap();
    let endpoint = tree.0.join("class/wwan/wwan0at0/device");
    fs::create_dir_all(&endpoint).unwrap();
    fs::write(endpoint.join("vendor"), "0x14c3").unwrap();
    fs::write(endpoint.join("device"), "0x4d75").unwrap();
    let rows = scan_sysfs(&tree.0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["port"], "/dev/wwan0at0");
    assert_eq!(rows[0]["bus"], "pci");
    assert_eq!(rows[0]["identityVerified"], false);
    assert_eq!(rows[0]["channelReady"], false);
}
#[test]
fn named_configuration_limits_and_id_length_match_sampler_bounds() {
    assert!(parse_owned_config(&OWN.replace("modem1", &"x".repeat(123))).is_err());
    let mut raw = String::new();
    for index in 0..17 {
        raw.push_str(&format!("network_panel.m{index}=modem\nnetwork_panel.m{index}.at_port='/dev/ttyUSB{index}'\nnetwork_panel.m{index}.model='mt5700m-cn'\nnetwork_panel.m{index}.transport='dedicated'\n"));
    }
    assert!(parse_owned_config(&raw).is_err());
}
#[cfg(unix)]
#[test]
fn sysfs_escape_symlinks_are_not_followed_as_modem_metadata() {
    let tree = Tree::new();
    let other = Tree::new();
    let external = other.usb("ttyUSB1", "12d1");
    fs::create_dir(tree.0.join("class/tty/ttyUSB1")).unwrap();
    std::os::unix::fs::symlink(external, tree.0.join("class/tty/ttyUSB1/device")).unwrap();
    assert!(scan_sysfs(&tree.0).unwrap().is_empty());
}
