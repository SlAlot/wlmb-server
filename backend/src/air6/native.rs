//! 软银 Air 6 的 MT6990 数据会话及 RIL 状态读取。
use super::PROVIDER;
use crate::{
    PanelError, Result,
    events::wall_ms,
    sampler::Source,
    transport::{Bus, NativeBus, bounded_read, process},
};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
    thread,
    time::Duration,
};

pub const PROTOCOL: &str = "wlmb_air6";
const INTERFACE: &str = "network.interface.modem5g";
const SESSION: &str = "/tmp/wlmb-air6-cid";
const UCI: &str = "/sbin/uci";
const MIPC: &str = "/usr/bin/mipc_wan_cli";

fn error(code: &'static str) -> PanelError {
    PanelError::new(code, 503)
}

pub fn detected() -> bool {
    bounded_read(Path::new("/tmp/sysinfo/board_name"), 128)
        .is_ok_and(|name| name.trim() == "softbank,air6")
        && Path::new(MIPC).is_file()
}

pub struct Air6;

impl Source for Air6 {
    fn modems(&self) -> Result<Vec<Value>> {
        Ok(vec![json!({
            "id":"air6-integrated", "model":"RG620T-SBK", "manufacturer":"quectel",
            "platform":"mt6990", "port":"MIPC", "instance":"a775",
            "infoTransport":PROVIDER, "infoReadOnly":true, "shared":false,
            "controlReadOnly":true, "smsReadSupported":false
        })])
    }

    fn query(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String> {
        Err(PanelError::new("AT_UNSUPPORTED", 422))
    }

    fn info_snapshot(&self, _: &Value) -> Result<Option<Value>> {
        let raw = process("/usr/bin/air6-nw-api", &[], None, Duration::from_secs(8))
            .map_err(|_| error("AIR6_RIL_UNAVAILABLE"))?;
        let value = serde_json::from_str(&raw).map_err(|_| error("AIR6_RIL_UNAVAILABLE"))?;
        Ok(Some(snapshot(&value)))
    }
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn band(value: &Value, rat: &str) -> Option<String> {
    if let Some(label) = value.as_str() {
        return Some(label.to_owned());
    }
    let prefix = if rat == "NR" { "n" } else { "B" };
    value
        .as_u64()
        .filter(|value| *value != 0)
        .map(|value| format!("{prefix}{value}"))
}

fn snapshot(value: &Value) -> Value {
    let at = value["UPDATED"]
        .as_u64()
        .map_or_else(wall_ms, |v| v.saturating_mul(1000));
    let mode = match value["RAT"].as_str() {
        Some("NR5G-SA") => "5G SA",
        Some("NR5G-NSA") => "5G NSA",
        Some("LTE") => "4G LTE",
        _ => "",
    };
    let cell = |row: &Value, role: &str| {
        let plmn = row["plmn"]
            .as_str()
            .filter(|v| (5..=6).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit()));
        let rat = if row["rat"] == "NR" || (row["rat"].is_null() && mode.starts_with("5G")) {
            "NR"
        } else {
            "LTE"
        };
        json!({
            "rat":rat, "role":role, "networkMode":mode,
            "band":band(row.get("band").unwrap_or_else(|| if row.is_null() { &value["BAND"] } else { &Value::Null }), rat),
            "channel":number(row.get("arfcn").unwrap_or_else(|| if row.is_null() { &value["ARFCN"] } else { &Value::Null })),
            "pci":number(row.get("pci").unwrap_or_else(|| if row.is_null() { &value["PCI"] } else { &Value::Null })),
            "rsrp":number(row.get("rsrp").unwrap_or_else(|| if row.is_null() { &value["RSRP"] } else { &Value::Null })),
            "rsrq":number(row.get("rsrq").unwrap_or_else(|| if row.is_null() { &value["RSRQ"] } else { &Value::Null })),
            "sinr":number(row.get("sinr").unwrap_or_else(|| if row.is_null() { &value["SINR"] } else { &Value::Null })),
            "rssi":number(row.get("rssi").unwrap_or(&Value::Null)),
            "mcc":plmn.map(|v| &v[..3]), "mnc":plmn.map(|v| &v[3..]),
            "source":PROVIDER,"observedAtMs":at,"stale":false
        })
    };
    let cells = value["cells"].as_array().into_iter().flatten();
    let mut seen = std::collections::HashSet::new();
    let mut serving: Vec<_> = cells
        .clone()
        .filter(|row| row["role"] == "serving")
        .map(|row| cell(row, "pcc"))
        .collect();
    let neighbors: Vec<_> = cells
        .filter(|row| row["role"] == "neighbor")
        .filter(|row| {
            seen.insert((
                row["rat"].as_str(),
                row["pci"].as_u64(),
                row["arfcn"].as_u64(),
            ))
        })
        .map(|row| cell(row, "neighbor"))
        .collect();
    if serving.is_empty() && value["REGISTERED"] == "YES" && !value["BAND"].is_null() {
        serving.push(cell(&Value::Null, "pcc"));
    }
    let neighbor_state = if value["api_errors"]["cells"] == 0 {
        "reported"
    } else {
        "unavailable"
    };
    json!({
        "source":PROVIDER, "observedAtMs":at, "stale":false,
        "partial":serving.is_empty(), "serving":serving, "carriers":serving, "neighbors":neighbors,
        "carrierState":{"status":"serving-only","inventoryComplete":false,"observedAtMs":at},
        "neighborState":{"status":neighbor_state,"observedAtMs":at},
        "operator":value["OPERATOR"], "simStatus":value["SIM"], "registration":value["REGISTRATION"],
        "temperature":number(&value["TEMP"]).map(|celsius| json!({"celsius":celsius,"source":PROVIDER,"observedAtMs":at})),
        "errors":[]
    })
}

fn interface(bus: &dyn Bus, method: &str) -> Result<Value> {
    bus.call(INTERFACE, method, json!({}), Duration::from_secs(2))
}

fn configured(bus: &dyn Bus) -> Result<Value> {
    let status = interface(bus, "status")?;
    if status["proto"] != PROTOCOL {
        return Err(error("AIR6_DIAL_UNAVAILABLE"));
    }
    Ok(status)
}

fn uci_value(option: &str) -> String {
    process(UCI, &["-q", "get", option], None, Duration::from_secs(2))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

pub fn status(bus: &dyn Bus) -> Result<Value> {
    let status = configured(bus)?;
    Ok(json!({
        "up":status["up"], "pending":status["pending"], "device":status["l3_device"],
        "ipv4":status["ipv4-address"], "ipv6":status["ipv6-address"],
        "errors":status["errors"],
        "apn":uci_value("network.modem5g.apn"),
        "ipType":uci_value("network.modem5g.iptype").parse::<u8>().unwrap_or(3),
        "automaticApn":uci_value("network.modem5g.auto_conf") != "0"
    }))
}

fn valid_apn(apn: &str) -> bool {
    apn.len() <= 100
        && apn
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
}

pub enum Operation {
    Connect,
    Disconnect,
    Apply {
        apn: String,
        ip_type: u8,
        auto: bool,
    },
}

impl Operation {
    pub fn parse(body: &Value) -> Result<Self> {
        let invalid = || PanelError::new("INVALID_REQUEST", 400);
        match body["action"].as_str() {
            Some("connect") => Ok(Self::Connect),
            Some("disconnect") => Ok(Self::Disconnect),
            Some("apply") => {
                let apn = body["apn"]
                    .as_str()
                    .map(str::trim)
                    .filter(|v| valid_apn(v))
                    .ok_or_else(invalid)?;
                let ip_type = body["ipType"]
                    .as_u64()
                    .filter(|v| (1..=3).contains(v))
                    .ok_or_else(invalid)? as u8;
                let auto = body["automaticApn"].as_bool().ok_or_else(invalid)?;
                if !auto && apn.is_empty() {
                    return Err(invalid());
                }
                Ok(Self::Apply {
                    apn: apn.to_owned(),
                    ip_type,
                    auto,
                })
            }
            _ => Err(invalid()),
        }
    }

    pub fn run(self, bus: &dyn Bus) -> Result<Value> {
        let _lock = lock("/var/lock/wlmb-air6-control.lock")?;
        let state = configured(bus)?;
        match self {
            Self::Connect => {
                interface(bus, "up")?;
            }
            Self::Disconnect => {
                interface(bus, "down")?;
            }
            Self::Apply { apn, ip_type, auto } => {
                let batch = format!(
                    "set network.modem5g.apn={apn:?}
set network.modem5g.iptype={ip_type}
set network.modem5g.auto_conf={}
commit network
",
                    u8::from(auto),
                );
                process(
                    UCI,
                    &["-q", "batch"],
                    Some(batch.as_bytes()),
                    Duration::from_secs(3),
                )?;
                if state["up"] == true || state["pending"] == true {
                    interface(bus, "down")?;
                }
                bus.call("network", "reload", json!({}), Duration::from_secs(3))?;
                interface(bus, "up")?;
            }
        }
        Ok(json!({"accepted":true}))
    }
}

#[allow(deprecated)]
fn lock(path: &str) -> Result<File> {
    use nix::fcntl::{FlockArg, flock};
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| error("AIR6_DIAL_UNAVAILABLE"))?;
    flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock)
        .map_err(|_| PanelError::new("AIR6_DIAL_BUSY", 409))?;
    Ok(file)
}

fn mipc(args: &[&str], seconds: u64) -> Result<String> {
    process(MIPC, args, None, Duration::from_secs(seconds))
        .map_err(|_| error("AIR6_MODEM_COMMAND_FAILED"))
}

fn ready() -> bool {
    let modem = bounded_read(Path::new("/sys/kernel/ccci/boot"), 256)
        .is_ok_and(|v| v.trim_start().starts_with("md1:4"));
    modem
        && Path::new("/dev/ttyCMIPC6").exists()
        && [
            "ccci_fsd",
            "ccci_mdinit",
            "ccci_rpcd_com",
            "mtk_netagent",
            "scd",
            "ql_netd",
            "ql_ril_service",
            "atcid",
        ]
        .iter()
        .all(|name| {
            process(
                "/bin/busybox",
                &["pidof", name],
                None,
                Duration::from_secs(1),
            )
            .is_ok()
        })
}

fn scalar(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|v| v.to_string()))
}

fn option(config: &Value, key: &str, default: &str) -> String {
    scalar(&config[key])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn bounded_option(value: &Value, low: u64, high: u64, default: u64) -> u64 {
    scalar(value)
        .and_then(|v| v.parse().ok())
        .filter(|n| (low..=high).contains(n))
        .unwrap_or(default)
}

fn cid(reply: &Value) -> Option<String> {
    scalar(&reply["cid"])
        .filter(|v| !v.is_empty() && v.len() <= 5 && v.bytes().all(|b| b.is_ascii_digit()))
}

fn notify(bus: &dyn Bus, payload: Value) -> Result<Value> {
    bus.call(
        "network.interface",
        "notify_proto",
        payload,
        Duration::from_secs(3),
    )
}

fn ip(value: &Value) -> Option<IpAddr> {
    value
        .as_str()?
        .parse::<IpAddr>()
        .ok()
        .filter(|v| !v.is_unspecified())
}

fn network_update(reply: &Value, cid: &str, config: &Value) -> Result<Value> {
    let device = reply["ifname"]
        .as_str()
        .filter(|name| {
            name.strip_prefix("ccmni").is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix.len() <= 3
                    && suffix.bytes().all(|b| b.is_ascii_digit())
            })
        })
        .ok_or_else(|| error("AIR6_DATA_CALL_INVALID"))?;
    let mut payload = json!({
        "interface":"modem5g", "action":0, "ifname":device, "link-up":true, "keep":true,
        "data":{"cid":cid}
    });
    let mut addresses = 0;
    let defaultroute =
        scalar(&config["defaultroute"]).as_deref() != Some("0") && config["defaultroute"] != false;
    let peerdns = scalar(&config["peerdns"]).as_deref() != Some("0") && config["peerdns"] != false;
    if let Some(IpAddr::V4(address)) = ip(&reply["v4_addr"]) {
        let mask = reply["v4_mask"]
            .as_str()
            .and_then(|s| s.parse::<Ipv4Addr>().ok())
            .and_then(|v| {
                let bits = u32::from(v);
                let count = bits.leading_ones();
                (bits == u32::MAX.checked_shl(32 - count).unwrap_or(0)).then_some(count)
            })
            .map(u64::from)
            .unwrap_or_else(|| bounded_option(&reply["v4_mask"], 0, 32, 32));
        payload["ipaddr"] = json!([{"ipaddr":address,"mask":mask.to_string()}]);
        addresses += 1;
        if let Some(IpAddr::V4(gw)) = ip(&reply["v4_gw"]).filter(|_| defaultroute) {
            payload["routes"] =
                json!([{"target":"0.0.0.0","netmask":"0","gateway":gw,"source":address}]);
        }
    }
    if let Some(IpAddr::V6(address)) = ip(&reply["v6_addr"]) {
        payload["ip6addr"] = json!([{"ipaddr":address,"mask":"128"}]);
        addresses += 1;
        let prefix = bounded_option(&reply["v6_prefix"], 1, 128, 0);
        if prefix != 0 {
            let net = Ipv6Addr::from(u128::from(address) & (u128::MAX << (128 - prefix)));
            payload["ip6prefix"] = json!([format!("{net}/{prefix}")]);
        }
        if let Some(IpAddr::V6(gw)) = ip(&reply["v6_gw"]).filter(|_| defaultroute) {
            payload["routes6"] = json!([{"target":"::","netmask":"0","gateway":gw}]);
        }
    }
    if addresses == 0 {
        return Err(error("AIR6_DATA_CALL_INVALID"));
    }
    if peerdns {
        let dns: Vec<_> = ["v4_dns1", "v4_dns2", "v6_dns1", "v6_dns2"]
            .iter()
            .filter_map(|key| ip(&reply[*key]))
            .collect();
        payload["dns"] = json!(dns);
    }
    Ok(payload)
}

fn stop_call(bus: &dyn Bus) -> Result<()> {
    let cached = fs::read_to_string(SESSION).ok().map(|s| json!({"cid":s}));
    let current = cached
        .as_ref()
        .and_then(cid)
        .or_else(|| interface(bus, "status").ok().and_then(|v| cid(&v["data"])));
    if let Some(id) = current {
        mipc(&["--data_call_deact", &id], 12)?;
        let _ = fs::remove_file(SESSION);
    }
    Ok(())
}

fn connect(bus: &dyn Bus, config: &Value) -> Result<()> {
    // 板级服务由 procd 启动，先等待 CCCI 和 RIL 完成初始化。
    for _ in 0..30 {
        if ready() {
            break;
        }
        thread::sleep(Duration::from_secs(2));
    }
    if !ready() {
        return Err(error("AIR6_MODEM_NOT_READY"));
    }
    stop_call(bus)?;
    mipc(&["--nw_radio_state_set", "1"], 8)?;
    let mut sim_ready = false;
    for _ in 0..8 {
        let sim = mipc(&["--show_sim_status"], 6).unwrap_or_default();
        match sim.trim() {
            "MIPC_SIM_STATUS_COMPLETE_READY" => {
                sim_ready = true;
                break;
            }
            "MIPC_SIM_STATUS_SIM_PIN" | "MIPC_SIM_STATUS_SIM_PUK" => {
                return Err(error("AIR6_SIM_LOCKED"));
            }
            _ => thread::sleep(Duration::from_secs(3)),
        }
    }
    if !sim_ready {
        return Err(error("AIR6_SIM_NOT_READY"));
    }
    let provision = if option(config, "auto_conf", "1") == "1" {
        mipc(&["--apn_provision_by_sim"], 10)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let apn = provision["apn"]
        .as_str()
        .filter(|v| !v.is_empty() && valid_apn(v))
        .map(str::to_owned)
        .unwrap_or_else(|| option(config, "apn", "0"));
    if !valid_apn(&apn) {
        return Err(error("AIR6_DATA_CALL_INVALID"));
    }
    let auth = bounded_option(
        &provision["auth_type"],
        0,
        3,
        bounded_option(&config["auth"], 0, 3, 0),
    );
    let username = option(&provision, "username", &option(config, "username", ""));
    let password = option(&provision, "password", &option(config, "password", ""));
    let ip_type = bounded_option(&config["iptype"], 1, 3, 3);
    let roaming = bounded_option(
        &provision["roaming_protocol"],
        1,
        3,
        bounded_option(&config["roamingtype"], 1, 3, 3),
    );
    let mtu = bounded_option(&config["mtu"], 1280, 1500, 1400);
    let ia_apn = option(&provision, "iaapn", &option(config, "iaapn", &apn));
    let ia_type = option(&provision, "iaprotocol", &ip_type.to_string());
    let ia_roaming = option(&provision, "iaroaming_protocol", &roaming.to_string());
    let rat = option(config, "rattype", "0");
    let data_roaming = option(config, "dataroaming", "0");
    let auth_arg = auth.to_string();
    let args = json!({
        "apn":apn, "ip_type":ip_type, "roaming_type":roaming, "auth_type":auth, "username":username,
        "password":password, "bearer_bitmask":"0xfffdffff", "mtu":mtu, "mode":1
    })
    .to_string();
    let _ = mipc(&["--data_call_deact_apn", &apn], 8);
    for _ in 0..3 {
        let _ = mipc(
            &[
                "--check_ia",
                &ia_apn,
                &rat,
                &ia_type,
                &ia_roaming,
                &username,
                &password,
                &auth_arg,
                "0xfffdffff",
                &data_roaming,
            ],
            10,
        );
        let _ = mipc(&["--check_nw_status"], 10);
        let reply = mipc(&["--data_call_act", &args], 25)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        if let Some(reply) = reply.filter(|v| scalar(&v["result"]).as_deref() == Some("0")) {
            let Some(id) = cid(&reply) else {
                let _ = mipc(&["--data_call_deact_apn", &apn], 8);
                return Err(error("AIR6_DATA_CALL_INVALID"));
            };
            // 先记录 CID，供取消或 netifd 重启后的清理使用。
            let result = (|| {
                let pending = format!("{SESSION}.new");
                fs::write(&pending, &id)
                    .and_then(|()| fs::rename(&pending, SESSION))
                    .map_err(|_| error("AIR6_DIAL_UNAVAILABLE"))?;
                let update = network_update(&reply, &id, config)?;
                let device = update["ifname"]
                    .as_str()
                    .ok_or_else(|| error("AIR6_DATA_CALL_INVALID"))?;
                if !Path::new("/sys/class/net").join(device).exists() {
                    return Err(error("AIR6_DATA_CALL_INVALID"));
                }
                let mtu = bounded_option(&reply["v4_mtu"], 1280, 1500, mtu);
                process(
                    "/sbin/ip",
                    &["link", "set", "dev", device, "mtu", &mtu.to_string()],
                    None,
                    Duration::from_secs(3),
                )
                .map_err(|_| error("AIR6_DATA_CALL_INVALID"))?;
                notify(bus, update)?;
                Ok(())
            })();
            if result.is_err() && mipc(&["--data_call_deact", &id], 12).is_ok() {
                let _ = fs::remove_file(SESSION);
            }
            return result;
        }
        thread::sleep(Duration::from_secs(3));
    }
    let _ = mipc(&["--data_call_deact_apn", &apn], 8);
    Err(error("AIR6_DATA_CALL_FAILED"))
}

pub fn netifd(action: &str, interface_name: &str, config: &str) -> Result<()> {
    if interface_name != "modem5g" || !detected() {
        return Err(error("AIR6_DIAL_UNAVAILABLE"));
    }
    let bus = NativeBus::default();
    if action == "teardown" {
        let _lock = lock("/var/lock/luci-modem5g-control.lock")?;
        let result = stop_call(&bus);
        notify(
            &bus,
            json!({"action":0,"interface":"modem5g","link-up":false}),
        )?;
        return result;
    }
    if action != "setup" {
        return Err(PanelError::new("INVALID_REQUEST", 400));
    }
    let config =
        serde_json::from_str::<Value>(config).map_err(|_| error("AIR6_DATA_CALL_INVALID"))?;
    let result =
        lock("/var/lock/luci-modem5g-control.lock").and_then(|_lock| connect(&bus, &config));
    if let Err(e) = &result {
        let _ = notify(
            &bus,
            json!({"action":3,"interface":"modem5g","error":[e.code]}),
        );
    }
    result
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn data_call_carries_both_families_and_validates_device() {
        let mut reply = json!({
            "ifname":"ccmni0","cid":3,"v4_addr":"10.0.0.2","v4_mask":"255.255.255.252",
            "v4_gw":"10.0.0.1","v4_dns1":"1.1.1.1","v6_addr":"2001:db8:1:2::abc",
            "v6_prefix":"64","v6_gw":"fe80::1","v6_dns1":"2606:4700:4700::1111"
        });
        let update = network_update(&reply, &cid(&reply).unwrap(), &json!({})).unwrap();
        assert_eq!(update["ipaddr"][0]["mask"], "30");
        assert_eq!(update["ip6prefix"][0], "2001:db8:1:2::/64");
        assert_eq!(update["routes6"][0]["gateway"], "fe80::1");
        assert_eq!(update["data"]["cid"], "3");
        assert_eq!(update["dns"].as_array().unwrap().len(), 2);
        let update =
            network_update(&reply, "3", &json!({"defaultroute":false,"peerdns":false})).unwrap();
        assert!(
            update["routes"].is_null() && update["routes6"].is_null() && update["dns"].is_null()
        );
        reply["ifname"] = json!("br-lan");
        assert!(network_update(&reply, "3", &json!({})).is_err());
    }

    #[test]
    fn neighbors_keep_unknown_metrics_and_are_deduplicated() {
        let neighbor = json!({"rat":"NR","role":"neighbor","pci":3,"arfcn":646666});
        let radio = snapshot(&json!({
            "RAT":"NR5G-SA","BAND":"n78","RSRP":"-93","REGISTERED":"YES","api_errors":{"cells":0},
            "cells":[{"rat":"NR","role":"serving","band":78,"rsrp":-93},neighbor,neighbor]
        }));
        assert_eq!(radio["serving"][0]["band"], "n78");
        assert_eq!(radio["serving"][0]["rsrp"], -93.0);
        assert_eq!(radio["neighbors"].as_array().unwrap().len(), 1);
        assert!(radio["neighbors"][0]["band"].is_null());
        assert!(radio["neighbors"][0]["rsrp"].is_null());
    }

    #[test]
    fn manual_apn_requires_a_safe_nonempty_name() {
        for apn in ["", "bad apn", "x\ncommit", "x;uci"] {
            assert!(
                Operation::parse(
                    &json!({"action":"apply","apn":apn,"ipType":3,"automaticApn":false})
                )
                .is_err()
            );
        }
        assert!(
            Operation::parse(
                &json!({"action":"apply","apn":"cmnet","ipType":3,"automaticApn":false})
            )
            .is_ok()
        );
    }
}
