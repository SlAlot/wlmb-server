#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use wlmb_server::temperature;
#[path = "support/temperature_cases.rs"]
mod cases;

#[test]
fn every_registered_temperature_path_has_bounded_ttl_and_a_platform_parser() {
    let mut supported = 0;
    for (_, models) in wlmb_server::catalog::MODELS {
        for model in models.split_whitespace() {
            let queries = temperature::queries(model);
            if queries.is_empty() {
                assert!(["n510m", "air724ug"].contains(&model));
                continue;
            }
            supported += 1;
            let profile = wlmb_server::catalog::identify(model);
            for query in queries {
                assert!(wlmb_server::security::read_only(query), "{model} {query}");
                assert_eq!(
                    profile["queries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|q| *q == query)
                        .count(),
                    1
                );
                let (raw, expected) = cases::reply(model, query);
                assert_eq!(
                    temperature::response(model, query, raw).unwrap()["celsius"],
                    expected,
                    "{model} {query}"
                );
                if *query != "AT!GSTATUS?" {
                    assert_eq!(wlmb_server::catalog::query_ttl(query, 2, false), 60);
                    assert!(temperature::slow_query(query));
                }
                assert!(temperature::response(model, query, &raw.replace("OK", "ERROR")).is_none());
                assert!(
                    temperature::response(model, query, &raw.replace("OK", "+CMS ERROR: 500\nOK"))
                        .is_none()
                );
                assert!(temperature::response(model, query, &raw.replace("OK", "")).is_none());
                assert!(temperature::response(model, query, &format!("{raw}{raw}")).is_none());
            }
        }
    }
    assert_eq!(supported, 89);
    assert!(temperature::queries("unknown-modem").is_empty());
}

#[test]
fn vendor_values_have_explicit_sensor_units_and_do_not_borrow_other_platform_values() {
    assert_eq!(
        temperature::modem("rm500u-cnv", cases::reply("rm500u-cnv", "AT+QTEMP").0).unwrap()["celsius"],
        50.
    );
    assert_eq!(
        temperature::modem("fm350-gl", "+GTSENRDTEMP: 1,43.5\nOK\n").unwrap()["celsius"],
        43.5
    );
    assert!(temperature::modem("fm350-gl", "+GTSENRDTEMP: 1,65535\nOK\n").is_none());
    assert!(temperature::modem("fm350-gl", "+GTSENRDTEMP: 1,255\nOK\n").is_none());
    assert!(temperature::modem("srm821", "+TEMP: \"cpu0-0-usr\",44\nOK\n").is_none());
    assert!(temperature::modem("srm825", "+TEMP: \"soc-thmzone\",45000\nOK\n").is_none());
    assert!(temperature::modem("t99w640", "TSENS: NaNC\nOK\n").is_none());
    assert!(
        temperature::modem("fn990a40", "#TEMPSENS: TSENS,45\n#TEMPSENS: TSENS,46\nOK\n").is_none()
    );
    assert!(
        temperature::modem("fm160-cn", "+GTLADC: cpu 45000\n+GTLADC: cpu 46000\nOK\n").is_none()
    );
    assert!(
        temperature::response(
            "fm350-gl",
            "AT+QTEMP",
            cases::reply("rm520n-gl", "AT+QTEMP").0
        )
        .is_none()
    );
    assert_eq!(
        temperature::quectel("+QTEMP: 45\nOK\n").unwrap()["celsius"],
        45.
    );
}
#[test]
fn named_quectel_core_sensors_do_not_use_hotter_pa_or_host_cpu() {
    let raw = "+QTEMP: \"cpuss-usr\",110\n+QTEMP: \"pa-therm1-usr\",115\n+QTEMP: \"mdm-usr\",\"47\"\nOK\n";
    assert_eq!(temperature::quectel(raw).unwrap()["celsius"], 47.);
    assert!(temperature::modem("fm650-cn", raw).is_none());
    assert!(temperature::quectel("+QTEMP: \"pa-therm1-usr\",115\nOK\n").is_none());
    assert!(temperature::quectel("+QTEMP: \"mdm-usr\",-273\nOK\n").is_none());
    assert!(temperature::quectel("+QTEMP: \"mdm-usr\",47\n+QTEMP: \"mdm-usr\",48\nOK\n").is_none());
    assert!(temperature::quectel("+QTEMP: \"mdm-usr\",47\n").is_none());
}
#[test]
fn temperature_decimal_formats_and_exact_read_only_scope() {
    assert_eq!(
        temperature::quectel("+QTEMP: \"mdm-usr\",45.5\nOK\n").unwrap()["celsius"],
        45.5
    );
    assert!(temperature::quectel("+QTEMP: 45\n+QTEMP: \"mdm-usr\",46\nOK\n").is_none());
    assert_eq!(
        temperature::modem("em9190", "!PCTEMP: Temperature: 36.75 degrees C\nOK\n").unwrap()["celsius"],
        36.75
    );
    for command in [
        "AT+MTSM=0",
        "AT+MTSM=1,7",
        "AT#TEMPSENS=1",
        "AT+GTSENRDTEMP=0",
    ] {
        assert!(!wlmb_server::security::read_only(command));
    }
}
#[test]
fn module_temperature_and_slow_query_classification() {
    let raw = "!GSTATUS:\nCurrent Time: 2863 Temperature: 31\nOK\n";
    assert_eq!(temperature::modem("mc7354", raw).unwrap()["celsius"], 31.);
    assert!(temperature::sierra(&raw.replace("31", "255")).is_none());
    assert!(temperature::sierra(&raw.replace("OK", "Temperature: 32\nOK")).is_none());
    assert!(temperature::slow_query("AT+QTEMP"));
    assert!(!temperature::slow_query("AT!GSTATUS?"));
    assert_eq!(wlmb_server::catalog::query_ttl("AT+QTEMP", 2, false), 60);
}
#[test]
fn sysfs_units_and_invalid_values() {
    assert_eq!(temperature::milli_celsius("75466\n"), Some(75.466));
    for s in ["NaN", "65535000", "151000", "-40001"] {
        assert_eq!(temperature::milli_celsius(s), None);
    }
}
#[test]
fn mt5700_uses_modem_sensors_not_peri_or_host() {
    let v = temperature::mt5700("^CHIPTEMP: 368,364,362,365,990,980,380,400,380,400,360,370\r\nOK")
        .unwrap();
    assert_eq!(v["celsius"], 40.);
    assert_eq!(v["sensors"][0]["sensor"], "modem1");
    assert!(temperature::mt5700("^CHIPTEMP: 1,2,3,4,5,6").is_none());
    assert!(temperature::mt5700("^CHIPTEMP: 1,2,3,4,5,6,7,8,65535,65535,11,12").is_none());
}
#[test]
fn no_temperature_invented_from_truncated_or_duplicate_response() {
    let raw = "^CHIPTEMP: 1,2,3,4,5,6,7,8,65535,420,11,12";
    assert!(temperature::mt5700(raw).is_none());
    assert_eq!(
        temperature::mt5700(&format!("{raw}\nOK\n")).unwrap()["celsius"],
        42.
    );
    assert!(temperature::mt5700(&format!("{raw}\n{raw}")).is_none());
}
