use serde_json::{Value, json};
use wlmb_server::telemetry::traffic_snapshot;

#[test]
fn projected_counters_retain_zero_and_age_without_unrelated_status() {
    let system = json!({"observedAtMs":1000, "uptimeSeconds":11, "lan":{"devices":[1,2]},
        "network":{"interface":"wan", "rates":null,"source":"l3-interface-counter-delta",
        "sinceBootRxBytes":0,"sinceBootTxBytes":5,"extra":1}});
    let sample = traffic_snapshot(&system, 16_000);
    assert_eq!(sample["system"]["network"]["sinceBootRxBytes"], 0);
    assert_eq!(sample["system"]["network"]["rates"], Value::Null);
    assert_eq!(sample["system"]["stale"], false);
    assert!(sample["system"].get("lan").is_none());
    assert!(sample["system"]["network"].get("extra").is_none());
    for now in [999, 16_001] {
        assert_eq!(traffic_snapshot(&system, now)["system"]["stale"], true);
    }
    assert_eq!(
        traffic_snapshot(&Value::Null, 0)["system"]["network"],
        Value::Null
    );
}
