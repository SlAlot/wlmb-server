//! 短信编码、存储与发送回执核验；复用当前设置，不擅自切换模组模式。
use crate::{PanelError, Result};

fn response<'a>(raw: &'a str, command: &str, prefix: &str) -> Option<&'a str> {
    if raw.len() > crate::MAX_AT_RESPONSE || raw.contains('\0') {
        return None;
    }
    let mut lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if lines.last() != Some(&"OK") {
        return None;
    }
    lines.pop();
    if ["AT+CMGF?", "AT+CPMS?", "AT+CSCS?", "AT+CSDH?"].contains(&command) {
        lines.retain(|line| !crate::sms_notice::has_cmti(line));
    }
    if lines.first() == Some(&command) {
        lines.remove(0);
    }
    if lines.len() != 1 {
        return None;
    }
    lines[0].strip_prefix(prefix).map(str::trim)
}
pub fn pdu_mode(raw: &str) -> bool {
    mode(raw) == Some(0)
}
pub fn mode(raw: &str) -> Option<u8> {
    match response(raw, "AT+CMGF?", "+CMGF:") {
        Some("0") => Some(0),
        Some("1") => Some(1),
        _ => None,
    }
}
pub fn text_ready(charset: &str, details: &str) -> bool {
    response(charset, "AT+CSCS?", "+CSCS:") == Some("\"UCS2\"")
        && response(details, "AT+CSDH?", "+CSDH:") == Some("1")
}
#[derive(Clone, PartialEq, Eq)]
pub struct Storage {
    pub read: String,
    selections: Vec<String>,
}
pub fn storage(raw: &str) -> Result<Storage> {
    let unavailable = || PanelError::new("SMS_STORAGE_UNAVAILABLE", 422);
    let row = response(raw, "AT+CPMS?", "+CPMS:").ok_or_else(unavailable)?;
    let fields: Vec<_> = row.split(',').map(str::trim).collect();
    if ![6, 9].contains(&fields.len()) {
        return Err(unavailable());
    }
    let mut selections = vec![];
    for triple in fields.chunks_exact(3) {
        let name = triple[0]
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or_else(unavailable)?;
        if !["ME", "SM", "MT", "SR", "TA"].contains(&name) {
            return Err(unavailable());
        }
        let count = |s: &str| {
            (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .then(|| s.parse::<u32>().ok().filter(|n| *n <= 65535))
                .flatten()
        };
        let used = count(triple[1]).ok_or_else(unavailable)?;
        let total = count(triple[2]).ok_or_else(unavailable)?;
        if used > total {
            return Err(unavailable());
        }
        selections.push(name.to_owned());
    }
    Ok(Storage {
        read: selections[0].clone(),
        selections,
    })
}
pub fn storage_query(mut query: impl FnMut() -> Result<String>) -> Result<Storage> {
    let first = storage(&query()?);
    if first.is_ok() {
        return first;
    }
    storage(&query()?)
}
pub fn send_reference(raw: &str) -> Result<u32> {
    let unknown = || PanelError::new("SMS_SEND_UNKNOWN", 503);
    let row = response(raw, "", "+CMGS:").ok_or_else(unknown)?;
    let mut fields = row.split(',').map(str::trim);
    let mr = fields.next().ok_or_else(unknown)?;
    if mr.is_empty() || !mr.bytes().all(|b| b.is_ascii_digit()) {
        return Err(unknown());
    }
    let mr = mr
        .parse::<u32>()
        .ok()
        .filter(|n| *n <= 255)
        .ok_or_else(unknown)?;
    if let Some(ack) = fields.next() {
        let hex = ack
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or_else(unknown)?;
        if hex.is_empty()
            || hex.len() > 512
            || hex.len() % 2 != 0
            || !hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(unknown());
        }
    }
    if fields.next().is_some() {
        return Err(unknown());
    }
    Ok(mr)
}
pub fn send_reference_with_pdu_echo(raw: &str, pdu: &str) -> Result<u32> {
    if raw.len() > crate::MAX_AT_RESPONSE
        || pdu.is_empty()
        || pdu.len() % 2 != 0
        || pdu.len() > 1024
        || !pdu.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(PanelError::new("SMS_SEND_UNKNOWN", 503));
    }
    let mut lines = raw
        .lines()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .peekable();
    if lines
        .peek()
        .is_some_and(|line| line.trim_end_matches('\u{1a}').eq_ignore_ascii_case(pdu))
    {
        lines.next();
    }
    send_reference(&lines.collect::<Vec<_>>().join("\n"))
}
