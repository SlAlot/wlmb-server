//! 已核验MT5700的IMS配置格式；不推断其他模组存在相同短信开关。
use serde_json::{Value, json};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImsConfig(pub [u8; 3]);
pub fn supported(model: &str) -> bool {
    crate::catalog::normalize(model) == "mt5700m-cn"
}
pub fn parse(raw: &str) -> Option<ImsConfig> {
    let rows = crate::radio::records(raw, "^IMSSWITCH:");
    if rows.len() != 1 || rows[0].len() != 3 {
        return None;
    }
    let values: Option<Vec<u8>> = rows[0]
        .iter()
        .map(|s| s.parse::<u8>().ok().filter(|n| *n <= 1))
        .collect();
    let values = values?;
    Some(ImsConfig([values[0], values[1], values[2]]))
}
pub fn confirmed(raw: &str) -> Option<ImsConfig> {
    if raw.len() > crate::MAX_AT_RESPONSE
        || raw.lines().rfind(|s| !s.trim().is_empty())?.trim() != "OK"
        || raw.lines().any(|s| s.trim().contains("ERROR"))
    {
        return None;
    }
    parse(raw)
}
impl ImsConfig {
    pub fn command(&self, enabled: bool) -> String {
        format!(
            "AT^IMSSWITCH={},{},{}",
            u8::from(enabled),
            self.0[1],
            self.0[2]
        )
    }
    pub fn public(&self) -> Value {
        json!({"enabled":self.0[0]==1,"flags":self.0,"source":"AT^IMSSWITCH?","meaning":"IMS configuration, not proof of SMS delivery","activation":"unverified"})
    }
}
