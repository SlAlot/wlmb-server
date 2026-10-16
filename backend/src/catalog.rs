//! 参考QModem公开型号、平台与命令清单；识别映射不代表硬件功能全部通过。
use serde_json::{Value, json};
pub const MODELS: &[(&str, &str)] = &[
    (
        "tdtech",
        "s10 mt5700m-cn mt5710-cn mh5000-31 mh5000-82 mh5000-82m",
    ),
    (
        "quectel",
        "em05 em05-cn ep06 em12 em120k em160r-gl ag598eeu rg200u-cn rg801 rm500u-cn rm500u-cnv rm500u-ea rm500s-ce rm500q-cn rm510q-gl rm510q-glha rg500q-ea rm500q-ae rm500q-gl rm502q-ae rm502q-gl rm505q-ae rm520n-cn rm520n-eu rm520n-gl rm520f-gl rm520f-bl rm521f-gl rm530n-gl rm551e-gl rg501q-eu rg520n-eu rg520n-eb mh8001-eu rg520f-eu rg520f-eb rg520f-jo rg530f-na rg650v-eu ec20f ec25 eg25 ec21 ec200a",
    ),
    (
        "fibocom",
        "fm650-cn fm350-gl fm350-gl-00 fm350r-gl rw350r-gl fm150-ae fm160-cn fm160-eau fm160-na fm170-eau fm190w-gl l610-eu nl668 nl678",
    ),
    ("meig", "srm815 srm825 srm825n srm821"),
    ("sierra", "em9190 em9291 mc7354 mc7355"),
    ("telit", "fn990a40"),
    (
        "simcom",
        "simcom-a8200-serias a8200c-m2 a7605c simcom-d8200g simcom-sim8200ea-m2 simcom-sim8202g-m2 simcom-sim8380g-m2",
    ),
    ("foxconn", "t77w968 t99w175 t99w373 t99w368 t99w640"),
    ("thales", "mv32-w-a mv32-w-b"),
    ("neoway", "n510m"),
    ("gosuncn", "me3630-w gm800"),
    ("openluat", "air724ug"),
];
pub fn normalize(model: &str) -> String {
    model
        .to_ascii_lowercase()
        .replace('_', "-")
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect()
}
fn profile(family: &str) -> Value {
    let (queries, parser, state): (&[&str], Option<&str>, &str) = match family {
        "tdtech" => (
            &["AT^MONSC", "AT^HFREQINFO?", "AT^MONNC"],
            Some("tdtech"),
            "fixture-testing",
        ),
        "quectel" => (
            &[
                "AT+QENG=\"servingcell\"",
                "AT+QCAINFO",
                "AT+QENG=\"neighbourcell\"",
            ],
            Some("quectel"),
            "fixture-testing",
        ),
        "fibocom" => (&["AT+GTCCINFO?", "AT+GTCAINFO?"], None, "pending"),
        "meig" => (&["AT^CELLINFO=1"], None, "pending"),
        "sierra" => (&["AT!GSTATUS?"], None, "pending"),
        "telit" => (&["AT#CAINFOEXT?"], None, "pending"),
        "simcom" => (&["AT+CPSI?"], Some("simcom"), "fixture-testing-partial"),
        "gosuncn" => (&["AT+ZCELLINFO?"], None, "pending"),
        "openluat" => (&["AT+CCED=0,2"], None, "pending"),
        "generic" => (&["AT+CSQ"], Some("generic"), "limited"),
        _ => (&[], None, "pending"),
    };
    json!({"queries":queries,"parser":parser,"state":state})
}
pub fn identify(model: &str) -> Value {
    let key = normalize(model);
    for (family, models) in MODELS {
        for name in models.split_whitespace() {
            if key == name
                || key.strip_prefix("simcom-").unwrap_or(&key)
                    == name.strip_prefix("simcom-").unwrap_or(name)
            {
                let mut p = profile(family);
                p["model"] = json!(name);
                p["family"] = json!(family);
                if ["rg200u-cn", "rm500u-cn", "rm500u-cnv", "rm500u-ea"].contains(&name) {
                    p["parser"] = json!("quectel-unisoc");
                }
                if ["mh5000-82", "mh5000-82m"].contains(&name) {
                    p["queries"] = json!(["AT^MONSC", "AT^HFREQINFO?", "AT^CSERSSI?"]);
                    p["parser"] = json!("huawei-unisoc");
                    p["state"] = json!("fixture-testing-partial");
                }
                if name == "rg801" {
                    p["queries"] = json!(["AT+QENG=\"servingcell\"", "AT+QCAINFO"]);
                    p["parser"] = json!("quectel-rg801");
                    p["state"] = json!("fixture-testing-partial");
                }
                if name == "rg650v-eu" {
                    p["parser"] = json!("quectel-rg650");
                }
                if ["fm650-cn", "l610-eu", "nl678"].contains(&name) {
                    p["queries"] = json!(["AT+GTCCINFO?", "AT+GTCAINFO?"]);
                    p["parser"] = json!("fibocom-hex");
                    p["state"] = json!("fixture-testing-partial");
                }
                if *family == "meig" {
                    p["parser"] = json!("meig-cellinfo");
                    p["state"] = json!("fixture-testing-partial");
                }
                if *family == "foxconn" {
                    let plus = ["t99w368", "t99w640"].contains(&name);
                    p["queries"] = json!([if plus { "AT+debug?" } else { "AT^debug?" }]);
                    p["parser"] = json!(if plus {
                        "foxconn-plus"
                    } else {
                        "foxconn-caret"
                    });
                    p["state"] = json!("fixture-testing-partial");
                }
                if *family == "thales" {
                    p["queries"] = json!(["AT^DEBUG?"]);
                    p["parser"] = json!("thales-debug");
                    p["state"] = json!("fixture-testing-partial");
                }
                if *family == "telit" {
                    p["parser"] = json!("telit-cainfoext");
                    p["state"] = json!("fixture-testing-partial");
                }
                if *family == "neoway" {
                    p["queries"] = json!(["AT+NETDMSGEX"]);
                    p["parser"] = json!("neoway-netdmsgex");
                    p["state"] = json!("fixture-testing-partial");
                }
                if *family == "gosuncn" {
                    p["parser"] = json!(if name == "gm800" {
                        "gosuncn-qualcomm"
                    } else {
                        "gosuncn-lte"
                    });
                    if name == "me3630-w" {
                        p["queries"] = json!(["AT+ZCELLINFO?", "AT+CESQ", "AT+CSQ"]);
                    }
                    p["state"] = json!("fixture-testing-partial");
                }
                if ["fm350-gl", "fm350-gl-00", "fm350r-gl", "rw350r-gl"].contains(&name) {
                    p["queries"] = json!(["AT+GTCCINFO?", "AT+CESQ", "AT+GTCAINFO?"]);
                    p["parser"] = json!("fibocom-fm350");
                    p["state"] = json!("fixture-testing-partial");
                }
                if [
                    "fm150-ae",
                    "fm160-cn",
                    "fm160-eau",
                    "fm160-na",
                    "fm170-eau",
                    "fm190w-gl",
                ]
                .contains(&name)
                {
                    p["queries"] = json!(["AT+GTCCINFO?", "AT+GTCAINFO?"]);
                    p["parser"] = json!("fibocom-qualcomm");
                    p["state"] = json!("fixture-testing-partial");
                }
                if name == "mt5700m-cn" {
                    p["queries"] = json!([
                        "AT^MONSC",
                        "AT^HFREQINFO?",
                        "AT^MONNC",
                        "AT^DSAMBR=1",
                        "AT^DSAMBR=8",
                        "AT^IMSSWITCH?",
                        "AT^CHIPTEMP?"
                    ]);
                }
                if name == "nl668" {
                    p["queries"] = json!(["AT+GTCCINFO?"]);
                    p["parser"] = json!("fibocom-nl668");
                    p["state"] = json!("fixture-testing-partial");
                }
                if name == "air724ug" {
                    p["queries"] = json!(["AT+CCED=0,1", "AT+CCED=0,2"]);
                    p["parser"] = json!("openluat-air724");
                    p["state"] = json!("fixture-testing-partial");
                }
                if ["em9190", "em9291"].contains(&name) {
                    p["queries"] = json!(["AT!NRINFO?", "AT!LTEINFO?"]);
                    p["parser"] = json!(if name == "em9190" {
                        "sierra-em91"
                    } else {
                        "sierra-em92"
                    });
                    p["state"] = json!("fixture-testing-partial");
                }
                if ["mc7354", "mc7355"].contains(&name) {
                    p["queries"] = json!(["AT!GSTATUS?"]);
                    p["parser"] = json!("sierra-lte");
                    p["state"] = json!("fixture-testing-partial");
                }
                if let Some(queries) = p["queries"].as_array_mut() {
                    for command in crate::temperature::queries(name) {
                        if !queries.iter().any(|query| query == command) {
                            queries.push(json!(command));
                        }
                    }
                }
                p["platform"] = json!(platform(name));
                if p["parser"].is_string() && p["parser"] != "openluat-air724" {
                    let use_ambr = *family == "quectel"
                        && platform(name) == "qualcomm"
                        && (name.starts_with("rm5")
                            || name.starts_with("rg5")
                            || name == "mh8001-eu"
                            || name == "rg650v-eu");
                    if let Some(queries) = p["queries"].as_array_mut() {
                        if use_ambr {
                            queries.push(json!("AT+QNWCFG=\"nr5g_ambr\""));
                        }
                        queries.push(json!("AT+CGCONTRDP"));
                        queries.push(json!("AT+CGEQOSRDP"));
                    }
                }
                p["protocolKey"] = json!(format!("{family}/{}", platform(name)));
                if p["parser"].is_string() {
                    if let Some(queries) = p["queries"].as_array_mut() {
                        queries.push(json!("AT+COPS?"));
                    }
                }
                return p;
            }
        }
    }
    let mut p = profile("generic");
    p["model"] = json!(if key.is_empty() { "unknown" } else { &key });
    p["family"] = json!("generic");
    p["platform"] = json!("unknown");
    p["protocolKey"] = json!("generic/unknown");
    if let Some(queries) = p["queries"].as_array_mut() {
        queries.push(json!("AT+COPS?"));
    }
    p
}
pub fn query_ttl(command: &str, interval: u64, primary: bool) -> u64 {
    if primary {
        return 0;
    }
    if crate::temperature::slow_query(command) {
        return interval.max(60);
    }
    interval.max(match command {
        "AT^MONNC" | "AT+QENG=\"neighbourcell\"" | "AT!LTEINFO?" | "AT+CCED=0,2" => 6,
        "AT^DSAMBR=1" => 60,
        _ => 3,
    })
}

pub fn platform(model: &str) -> &'static str {
    let key = normalize(model);
    for (platform, models) in [
        ("hisilicon", "s10 mt5700m-cn mt5710-cn mh5000-31 rg801"),
        (
            "unisoc",
            "air724ug mh5000-82 mh5000-82m rg200u-cn rm500u-cn rm500u-cnv rm500u-ea fm650-cn l610-eu srm821 n510m",
        ),
        ("mediatek", "fm350-gl fm350-gl-00 fm350r-gl rw350r-gl"),
        ("asrmicro", "simcom-a8200-serias a8200c-m2"),
        ("lte12", "em120k em160r-gl"),
        (
            "lte",
            "em05 em05-cn ep06 em12 ec20f ec25 eg25 ec21 ec200a nl668 nl678 mc7354 mc7355 a7605c me3630-w",
        ),
    ] {
        if models
            .split_whitespace()
            .any(|model| normalize(model) == key)
        {
            return platform;
        }
    }
    if MODELS.iter().any(|(_, models)| {
        models
            .split_whitespace()
            .any(|model| normalize(model) == key)
    }) {
        "qualcomm"
    } else {
        "unknown"
    }
}

pub fn inventory() -> Value {
    let rows: Vec<Value> = MODELS.iter().flat_map(|(_, models)| models.split_whitespace()).map(|model| {
        let mut row = identify(model);
        let parser = row["parser"].as_str().unwrap_or("");
        let signal = if parser.is_empty() { "pending" } else { "fixture-partial" };
        row["platform"] = json!(platform(model));
        row["hardwareAccepted"] = json!(false);
        let radio_features = radio_capabilities(model);
        let qos_query = row["queries"].as_array().is_some_and(|queries| queries.iter().any(|q| q == "AT+CGEQOSRDP"));
        row["features"] = json!({
            "radio": signal,
            "neighbors": if radio_features["neighbors"] == true { "implemented-field-dependent" } else { "not-exposed-by-parser" },
            "carrierAggregation": if radio_features["carrierAggregation"] == true { "implemented-field-dependent" } else { "not-exposed-by-parser" },
            "bandLock": if crate::lock_protocol::band_supported(model) { "implemented-unverified-write" } else { "pending" },
            "cellLock": if crate::lock_protocol::cell_supported(model) { "implemented-unverified-write" } else { "pending" },
            "simAmbr": if model == "mt5700m-cn" || qos_query { "implemented-firmware-dependent" } else { "pending" },
            "sessionQos": if qos_query { "implemented-firmware-dependent" } else { "pending" },
            "temperature": if crate::temperature::query(model).is_some() { "implemented-firmware-dependent" } else { "pending" },
            "sms": "standard-pdu-needs-firmware-negotiation",
            "networkPreference":"pending",
            "cellularEnable":"pending",
            "smsSharedTransactions":"requires-reviewed-owner-contract",
            "smsNotifications":"bounded-owned-endpoint-cmti-or-poll-fallback"
        });
        row
    }).collect();
    json!({"schema":"WLMB-PROTOCOL-COVERAGE/1","runtimeDependency":"none","models":rows})
}

pub fn radio_capabilities(model: &str) -> Value {
    let p = identify(model);
    let parser = p["parser"].as_str().unwrap_or("");
    let implemented = !parser.is_empty();
    let summary_only = [
        "generic",
        "sierra-lte",
        "simcom",
        "meig-cellinfo",
        "neoway-netdmsgex",
        "gosuncn-lte",
        "gosuncn-qualcomm",
        "thales-debug",
    ]
    .contains(&parser);
    let no_neighbors = summary_only
        || [
            "huawei-unisoc",
            "foxconn-plus",
            "foxconn-caret",
            "telit-cainfoext",
        ]
        .contains(&parser)
        || normalize(model) == "rg801";
    json!({"signal":implemented,
        "neighbors":implemented && !no_neighbors,
        "carrierAggregation":implemented && !summary_only
            && !["fibocom-nl668", "openluat-air724"].contains(&parser)})
}
