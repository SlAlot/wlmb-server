//! HTTP／RPC入口校验：局域网来源、请求长度、JSON结构及允许的方法。
use crate::{API_PORT, MAX_BODY, PanelError, Result};
use serde_json::Value;
use std::net::Ipv4Addr;

pub const POST_ROUTES: &[&str] = &[
    "/api/login",
    "/api/logout",
    "/api/status",
    "/api/settings",
    "/api/diagnostics",
    "/api/refresh",
    "/api/at_prepare",
    "/api/at_execute",
    "/api/lock_status",
    "/api/lock_prepare",
    "/api/lock_execute",
    "/api/host_prepare",
    "/api/host_execute",
    "/api/control_result",
    "/api/sms_list",
    "/api/sms_settings_prepare",
    "/api/sms_settings_execute",
    "/api/sms_refresh",
    "/api/sms_read_state",
    "/api/sms_prepare",
    "/api/sms_execute",
    "/api/sms_forward_settings",
    "/api/sms_forward_configure",
];
pub fn safe_read(method: &str) -> bool {
    [
        "status",
        "settings",
        "diagnostics",
        "control_result",
        "sms_list",
        "sms_forward_settings",
    ]
    .contains(&method)
}
pub fn decode_json(raw: &[u8]) -> Result<Value> {
    if !(2..=MAX_BODY).contains(&raw.len()) {
        return Err(PanelError::new("BODY_TOO_LARGE", 413));
    }
    let value: Value =
        serde_json::from_slice(raw).map_err(|_| PanelError::new("INVALID_JSON", 400))?;
    if !value.is_object() {
        return Err(PanelError::new("INVALID_JSON", 400));
    }
    Ok(value)
}
#[derive(Clone, Copy)]
pub struct Lan {
    pub address: Ipv4Addr,
    pub mask: u8,
}
impl Lan {
    pub fn new(address: Ipv4Addr, mask: u8) -> Result<Self> {
        if !address.is_private() || !(8..=30).contains(&mask) {
            return Err(PanelError::new("LAN_UNAVAILABLE", 503));
        }
        Ok(Self { address, mask })
    }
    pub fn check(&self, peer: Ipv4Addr, headers: &[(String, String)]) -> Result<()> {
        let mask = u32::MAX << (32 - self.mask);
        if u32::from(peer) & mask != u32::from(self.address) & mask {
            return Err(PanelError::new("LAN_ONLY", 403));
        }
        let values = |key: &str| {
            headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_str())
                .collect::<Vec<_>>()
        };
        let expected = format!("{}:{}", self.address, API_PORT);
        let hosts = values("host");
        if hosts.len() != 1 || hosts[0] != expected {
            return Err(PanelError::new("HOST_DENIED", 403));
        }
        let origins = values("origin");
        if origins.len() > 1
            || origins
                .first()
                .is_some_and(|v| *v != format!("http://{expected}"))
        {
            return Err(PanelError::new("ORIGIN_DENIED", 403));
        }
        if values("sec-fetch-site")
            .iter()
            .any(|v| v.eq_ignore_ascii_case("cross-site"))
        {
            return Err(PanelError::new("ORIGIN_DENIED", 403));
        }
        Ok(())
    }
}
pub fn content_length(headers: &[(String, String)]) -> Result<usize> {
    let mut length = None;
    let mut content_type = None;
    for (key, value) in headers {
        if key.eq_ignore_ascii_case("transfer-encoding") {
            return Err(PanelError::new("INVALID_LENGTH", 400));
        }
        if key.eq_ignore_ascii_case("content-length") {
            if length.is_some() || value.is_empty() || !value.bytes().all(|v| v.is_ascii_digit()) {
                return Err(PanelError::new("INVALID_LENGTH", 400));
            }
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| PanelError::new("INVALID_LENGTH", 400))?,
            );
        }
        if key.eq_ignore_ascii_case("content-type") {
            if content_type.is_some() {
                return Err(PanelError::new("CONTENT_TYPE", 415));
            }
            content_type = Some(value);
        }
    }
    let length = length.ok_or_else(|| PanelError::new("INVALID_LENGTH", 400))?;
    if !(2..=MAX_BODY).contains(&length) {
        return Err(PanelError::new("BODY_TOO_LARGE", 413));
    }
    if !content_type.is_some_and(|v| {
        v.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
    }) {
        return Err(PanelError::new("CONTENT_TYPE", 415));
    }
    Ok(length)
}
