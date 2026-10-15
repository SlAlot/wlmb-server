//! 兼容文本模式短信列表与分段缓存，未知编码或不完整记录不伪造正文。
use crate::{
    PanelError, Result,
    sms::{Direction, MAX_MESSAGES, Message},
    sms_pdu::Record,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

fn invalid() -> PanelError {
    PanelError::new("SMS_TEXT_INVALID", 422)
}
fn unsupported() -> PanelError {
    PanelError::new("SMS_TEXT_ENCODING_UNSUPPORTED", 422)
}
#[derive(Default)]
pub struct Cache {
    scope: String,
    verified: BTreeMap<String, (u8, Instant)>,
}
impl Cache {
    pub fn bind(&mut self, scope: String) {
        if self.scope != scope {
            self.verified.clear();
            self.scope = scope;
        }
    }
    pub fn clear(&mut self) {
        self.verified.clear();
    }
}
fn decimal(raw: &str, high: u32) -> Result<u32> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    raw.parse::<u32>()
        .ok()
        .filter(|n| *n <= high)
        .ok_or_else(invalid)
}
fn fields(raw: &str) -> Result<Vec<(String, bool)>> {
    let mut out = vec![];
    let mut rest = raw.trim();
    loop {
        let (value, quoted, tail) = if let Some(tail) = rest.strip_prefix('"') {
            let end = tail.find('"').ok_or_else(invalid)?;
            (tail[..end].to_owned(), true, tail[end + 1..].trim_start())
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            let value = rest[..end].trim();
            if value.contains('"') {
                return Err(invalid());
            }
            (value.to_owned(), false, &rest[end..])
        };
        out.push((value, quoted));
        if out.len() > 12 {
            return Err(invalid());
        }
        if tail.is_empty() {
            return Ok(out);
        }
        rest = tail.strip_prefix(',').ok_or_else(invalid)?.trim_start();
    }
}
fn quoted(field: &(String, bool)) -> Result<&str> {
    if field.1 {
        Ok(&field.0)
    } else {
        Err(invalid())
    }
}
fn utf16(hex: &str, max_bytes: usize) -> Result<String> {
    if hex.is_empty()
        || hex.len() > max_bytes * 2
        || hex.len() % 4 != 0
        || !hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    let units = (0..hex.len())
        .step_by(4)
        .map(|at| u16::from_str_radix(&hex[at..at + 4], 16).map_err(|_| invalid()))
        .collect::<Result<Vec<_>>>()?;
    let value = String::from_utf16(&units).map_err(|_| invalid())?;
    if value.contains('\0') {
        return Err(invalid());
    }
    Ok(value)
}
fn optional_text(field: &(String, bool)) -> Result<()> {
    if !field.0.is_empty() {
        utf16(quoted(field)?, 96)?;
    }
    Ok(())
}
fn time(raw: &str) -> Result<String> {
    if !raw.is_ascii()
        || raw.len() != 20
        || &raw[2..3] != "/"
        || &raw[5..6] != "/"
        || &raw[8..9] != ","
        || &raw[11..12] != ":"
        || &raw[14..15] != ":"
        || !["+", "-"].contains(&&raw[17..18])
    {
        return Err(invalid());
    }
    let year = decimal(&raw[..2], 99)?;
    let year = if year >= 70 { 1900 + year } else { 2000 + year };
    let m = decimal(&raw[3..5], 12)?;
    let d = decimal(&raw[6..8], 31)?;
    let days = match m {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if m == 0 || d == 0 || d > days {
        return Err(invalid());
    }
    let h = decimal(&raw[9..11], 23)?;
    let min = decimal(&raw[12..14], 59)?;
    let sec = decimal(&raw[15..17], 59)?;
    let tz = decimal(&raw[18..20], 56)?;
    Ok(format!(
        "{year:04}-{m:02}-{d:02} {h:02}:{min:02}:{sec:02} {}{:02}:{:02}",
        &raw[17..18],
        tz / 4,
        (tz % 4) * 15
    ))
}
fn direction(field: &(String, bool)) -> Result<Direction> {
    match quoted(field)? {
        "REC READ" | "REC UNREAD" => Ok(Direction::Incoming),
        "STO SENT" => Ok(Direction::Outgoing),
        _ => Err(unsupported()),
    }
}
fn lines<'a>(raw: &'a str, echo: &str) -> Result<Vec<&'a str>> {
    if raw.len() > crate::MAX_AT_RESPONSE || raw.contains('\0') {
        return Err(invalid());
    }
    let mut lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if lines.last() != Some(&"OK") || lines.iter().filter(|s| **s == "OK").count() != 1 {
        return Err(invalid());
    }
    lines.pop();
    if lines.first() == Some(&echo) {
        lines.remove(0);
    }
    Ok(lines)
}
struct Candidate {
    message: Message,
    length: u32,
    toa: u32,
    body: String,
}
impl Candidate {
    fn key(&self) -> Result<String> {
        Ok(format!(
            "{}:{}:{}",
            self.message.fingerprint()?,
            self.toa,
            self.length
        ))
    }
}
fn list(raw: &str, modem: &str, storage: &str) -> Result<Vec<Candidate>> {
    let lines = lines(raw, "AT+CMGL=\"ALL\"")?;
    if lines.len() % 2 != 0 || lines.len() / 2 > MAX_MESSAGES {
        return Err(invalid());
    }
    let mut indexes = BTreeSet::new();
    let mut rows = vec![];
    for pair in lines.chunks_exact(2) {
        let p = fields(pair[0].strip_prefix("+CMGL:").ok_or_else(invalid)?)?;
        if ![6, 7].contains(&p.len()) {
            return Err(invalid());
        }
        let index = decimal(&p[0].0, 65535)?;
        if p[0].1 || !indexes.insert(index) {
            return Err(invalid());
        }
        let dir = direction(&p[1])?;
        optional_text(&p[3])?;
        let timestamp = if dir == Direction::Incoming {
            if p.len() != 7 {
                return Err(invalid());
            }
            time(quoted(&p[4])?)?
        } else {
            if p.len() == 7 && !p[4].0.is_empty() {
                return Err(invalid());
            }
            String::new()
        };
        let toa = decimal(&p[p.len() - 2].0, 255)?;
        if p[p.len() - 2].1 || toa & 128 == 0 {
            return Err(invalid());
        }
        let length = decimal(&p[p.len() - 1].0, 560)?;
        if p[p.len() - 1].1 || length as usize != pair[1].len() {
            return Err(invalid());
        }
        let message = Message {
            modem: modem.into(),
            storage: storage.into(),
            index,
            direction: dir,
            contact: utf16(quoted(&p[2])?, 96)?,
            timestamp,
            content: utf16(pair[1], 280)?,
        };
        message.validate()?;
        rows.push(Candidate {
            message,
            length,
            toa,
            body: pair[1].into(),
        });
    }
    Ok(rows)
}
fn detail(raw: &str, c: &Candidate) -> Result<u8> {
    let echo = format!("AT+CMGR={}", c.message.index);
    let lines = lines(raw, &echo)?;
    if lines.len() != 2 || lines[1] != c.body {
        return Err(invalid());
    }
    let p = fields(lines[0].strip_prefix("+CMGR:").ok_or_else(invalid)?)?;
    let incoming = c.message.direction == Direction::Incoming;
    if !(if incoming {
        p.len() == 11
    } else {
        [10, 11].contains(&p.len())
    }) || direction(&p[0])? != c.message.direction
        || utf16(quoted(&p[1])?, 96)? != c.message.contact
    {
        return Err(invalid());
    }
    if incoming && time(quoted(&p[3])?)? != c.message.timestamp {
        return Err(invalid());
    }
    optional_text(&p[2])?;
    let offset = if incoming { 4 } else { 3 };
    let ints = p[offset..offset + 4]
        .iter()
        .map(|p| {
            if p.1 {
                Err(invalid())
            } else {
                decimal(&p.0, 255)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let (toa, fo, _pid, dcs) = (ints[0], ints[1], ints[2], ints[3]);
    if toa != c.toa
        || fo & 3 != if incoming { 0 } else { 1 }
        || fo & 64 != 0
        || ![0, 8].contains(&dcs)
    {
        return Err(unsupported());
    }
    if !incoming {
        match (fo >> 3) & 3 {
            0 if p.len() == 10 => {}
            2 if p.len() == 11 && !p[7].1 && decimal(&p[7].0, 255).is_ok() => {}
            _ => return Err(unsupported()),
        }
    }
    if p.last()
        .is_none_or(|p| p.1 || decimal(&p.0, 560).ok() != Some(c.length))
    {
        return Err(invalid());
    }
    optional_text(&p[p.len() - 3])?;
    if p[p.len() - 2].1 || decimal(&p[p.len() - 2].0, 255)? & 128 == 0 {
        return Err(invalid());
    }
    if dcs == 8 {
        if c.body.len() > 280 {
            return Err(unsupported());
        }
    } else {
        const BASE: &str = "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà";
        const EXT: &str = "\u{c}^{}\\[~]|€";
        let mut septets = 0;
        for ch in c.message.content.chars() {
            septets += if BASE.contains(ch) {
                1
            } else if EXT.contains(ch) {
                2
            } else {
                return Err(unsupported());
            };
        }
        if septets > 160 {
            return Err(unsupported());
        }
    }
    Ok(dcs as u8)
}
pub fn read(
    mut query: impl FnMut(&str, u64) -> Result<String>,
    modem: &str,
    cache: &mut Cache,
) -> Result<Vec<Record>> {
    let charset = query("AT+CSCS?", 3)?;
    let details = query("AT+CSDH?", 3)?;
    if !crate::sms_session::text_ready(&charset, &details) {
        return Err(PanelError::new("SMS_TEXT_SETTINGS_REQUIRED", 422));
    }
    let before = crate::sms_session::storage_query(|| query("AT+CPMS?", 3))?;
    let rows = list(&query("AT+CMGL=\"ALL\"", 10)?, modem, &before.read)?;
    let keys = rows
        .iter()
        .map(Candidate::key)
        .collect::<Result<Vec<_>>>()?;
    cache
        .verified
        .retain(|key, (_, when)| keys.contains(key) && when.elapsed() < Duration::from_secs(1800));
    let until = Instant::now() + Duration::from_secs(4);
    let (mut checked, mut pending) = (0, false);
    let mut newly_verified = vec![];
    for (c, key) in rows.iter().zip(&keys) {
        if cache.verified.contains_key(key) {
            continue;
        }
        if checked >= 8 || Instant::now() >= until {
            pending = true;
            break;
        }
        let dcs = detail(&query(&format!("AT+CMGR={}", c.message.index), 1)?, c)?;
        newly_verified.push((key.clone(), (dcs, Instant::now())));
        checked += 1;
    }
    let after = crate::sms_session::storage_query(|| query("AT+CPMS?", 3))?;
    let mode = query("AT+CMGF?", 3)?;
    let charset = query("AT+CSCS?", 3)?;
    let details = query("AT+CSDH?", 3)?;
    if before != after
        || crate::sms_session::mode(&mode) != Some(1)
        || !crate::sms_session::text_ready(&charset, &details)
    {
        cache.clear();
        return Err(PanelError::new("SMS_STORAGE_CHANGED", 409));
    }
    cache.verified.extend(newly_verified);
    if pending {
        return Err(PanelError::new("SMS_TEXT_VERIFY_PENDING", 409));
    }
    rows.into_iter()
        .zip(keys)
        .map(|(c, key)| {
            Ok(Record {
                dcs: cache.verified.get(&key).ok_or_else(invalid)?.0,
                indexes: vec![c.message.index],
                message: c.message,
                concatenation: None,
            })
        })
        .collect()
}

pub fn scope(row: &serde_json::Value) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&serde_json::json!([
                row["id"],
                row["port"],
                row["model"],
                row["instance"]
            ]))
            .map_err(|_| invalid())?
        )
    ))
}
