#![cfg(unix)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
#![allow(clippy::unwrap_used)]
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use wlmb_server::{
    host_controls::{self, HostIo},
    native_host::NativeHost,
    private_store::PrivateStore,
    transport::{Bus, NativeBus},
};
struct Associated {
    status: Value,
    calls: AtomicUsize,
}
impl Bus for Associated {
    fn call(
        &self,
        object: &str,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> wlmb_server::Result<Value> {
        assert!(timeout <= Duration::from_secs(1));
        self.calls.fetch_add(1, Ordering::SeqCst);
        if object == "network.wireless" && method == "status" {
            return Ok(self.status.clone());
        }
        assert_eq!((object, method), ("iwinfo", "assoclist"));
        assert_eq!(
            params["device"], "wlan0",
            "never query guest/STA/injected ifnames"
        );
        Ok(json!({"results":[{"mac":"02:00:00:00:00:02"}]}))
    }
}

#[test]
#[ignore = "Writes isolated guest UCI and owned nft table; never a supply router"]
fn isolated_native_uci_private_store_and_owned_guard() {
    assert!(Path::new("/etc/network-panel-isolated-lab").is_file());
    assert!(!Path::new("/etc/config/qmodem").exists());
    assert!(
        !Path::new("/etc/config/wireless").exists(),
        "Lab must have no real wireless configuration"
    );
    let dir = std::path::PathBuf::from(format!("/tmp/wlmb-host-r3-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    struct Cleanup(std::path::PathBuf, Arc<NativeHost>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.1.clear_guard();
            let _ = fs::remove_file("/etc/config/wireless");
            for file in ["settings.json", "host.json", "traffic.json"] {
                let _ = fs::remove_file(self.0.join(file));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }
    let bus: Arc<dyn Bus> = Arc::new(NativeBus::default());
    let host = NativeHost::new(bus.clone(), PrivateStore::open(&dir).unwrap());
    let _cleanup = Cleanup(dir.clone(), host.clone());
    let original_network = bus
        .call(
            "uci",
            "get",
            json!({"config":"network"}),
            Duration::from_secs(2),
        )
        .unwrap();
    fs::write("/etc/config/wireless", "config wifi-device 'radio0'\n option type 'wlmb-lab-none'\n option band '2g'\n option disabled '1'\nconfig wifi-iface 'ap0'\n option device 'radio0'\n option mode 'ap'\n option network 'lan'\n option encryption 'psk2'\n option ssid 'Lab'\n option key 'LabSecret123'\n").unwrap();
    fs::set_permissions("/etc/config/wireless", fs::Permissions::from_mode(0o600)).unwrap();
    let snapshot = host.snapshot().unwrap();
    assert_eq!(host_controls::profiles(&snapshot["wireless"]).len(), 1);
    let change = host_controls::wifi_changes(&snapshot,&json!({"kind":"wifi_edit","band":"2g","changes":{"ssid":"Panel ' quoted 中文","password":"LabPass'1234"}})).unwrap();
    let applied = host.wifi(&snapshot, &[change]);
    if applied.is_err() {
        eprintln!(
            "lab committed file has new SSID={}",
            fs::read_to_string("/etc/config/wireless")
                .unwrap()
                .contains("quoted 中文")
        );
        let current = host.snapshot().unwrap();
        eprintln!(
            "lab readback SSID={:?}; keyMatches={}; networkMatches={}",
            current["wireless"]["values"]["ap0"]["ssid"],
            current["wireless"]["values"]["ap0"]["key"] == "LabPass'1234",
            current["network"] == original_network
        );
    }
    applied.unwrap();
    let current = host.snapshot().unwrap();
    assert_eq!(
        current["wireless"]["values"]["ap0"]["ssid"],
        "Panel ' quoted 中文"
    );
    assert_eq!(current["wireless"]["values"]["ap0"]["key"], "LabPass'1234");
    assert_eq!(
        current["network"], original_network,
        "No LAN/WAN/dialing configuration edited"
    );
    assert!(host.wifi(&snapshot, &[]).is_err());
    host.save("settings.json", b"{\"version\":1}").unwrap();
    assert_eq!(
        host.load("settings.json").unwrap(),
        Some(b"{\"version\":1}".to_vec())
    );
    assert_eq!(
        fs::metadata(dir.join("settings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(host.load("../../etc/shadow").is_err());
    for (cfg, up, ifname, found) in [
        (json!({"mode":"ap","network":["lan"]}), true, "wlan0", true),
        (json!({"mode":"ap","network":"guest"}), true, "wlan0", false),
        (json!({"mode":"sta","network":"lan"}), true, "wlan0", false),
        (json!({"mode":"ap","network":"lan"}), false, "wlan0", false),
        (
            json!({"mode":"ap","network":"lan","disabled":true}),
            true,
            "wlan0",
            false,
        ),
        (
            json!({"mode":"ap","network":"lan","disabled":"1"}),
            true,
            "wlan0",
            false,
        ),
        (
            json!({"mode":"ap","network":"lan","disabled":1}),
            true,
            "wlan0",
            false,
        ),
        (
            json!({"mode":"ap","network":"lan"}),
            true,
            "wlan0;reboot",
            false,
        ),
    ] {
        let bus = Arc::new(Associated {
            status: json!({"radio0":{"up":up,"interfaces":[{"ifname":ifname,"config":cfg}]}}),
            calls: AtomicUsize::new(0),
        });
        let native = NativeHost::new(bus.clone(), PrivateStore::open(&dir).unwrap());
        assert_eq!(
            native
                .known_device(&json!({"devices":[]}), "02:00:00:00:00:02")
                .unwrap(),
            found
        );
        let before = bus.calls.load(Ordering::SeqCst);
        assert!(
            native
                .known_device(
                    &json!({"devices":[{"mac":"02:00:00:00:00:02"}]}),
                    "02:00:00:00:00:02"
                )
                .unwrap()
        );
        assert_eq!(
            bus.calls.load(Ordering::SeqCst),
            before,
            "cached clients need no additional IO"
        );
    }
    let controlled = json!({"lanDevice":"br-lan","wanDevice":"wlmb-lab-wan","nftAvailable":true,"flowOffload":false});
    let blocked = BTreeSet::from(["02:00:00:00:00:01".into()]);
    host.guard(&controlled, &blocked, false).unwrap();
    assert!(host.guard_present().unwrap());
    host.guard(&controlled, &blocked, true).unwrap();
    assert!(host.guard_present().unwrap());
    host.clear_guard().unwrap();
    assert!(!host.guard_present().unwrap());
    assert!(
        host.snapshot().unwrap()["nftAvailable"] == true,
        "System fw4 remains available"
    );
}
