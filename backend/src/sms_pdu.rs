//! 标准短信PDU编解码、字符集与分段边界校验。
use crate::{
    PanelError, Result,
    sms::{Direction, MAX_MESSAGES, MAX_SMS_BYTES, Message},
};
use std::collections::{BTreeMap, BTreeSet};
fn invalid() -> PanelError {
    PanelError::new("SMS_PDU_INVALID", 422)
}
pub fn pdu_mode(raw: &str) -> bool {
    crate::sms_session::pdu_mode(raw)
}
pub fn preserve_error(error: PanelError) -> PanelError {
    if error.code == "AT_UNSUPPORTED" {
        PanelError::new("SMS_PRESERVE_UNSUPPORTED", 422)
    } else {
        error
    }
}
struct Cursor<'a> {
    raw: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or_else(invalid)?;
        let value = self.raw.get(self.at..end).ok_or_else(invalid)?;
        self.at = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
}
pub fn bytes(hex: &str) -> Result<Vec<u8>> {
    if hex.is_empty()
        || hex.len() > 512
        || hex.len() % 2 != 0
        || !hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| invalid()))
        .collect()
}
fn gsm(raw: &[u8], count: usize, offset: usize) -> Result<String> {
    const BASE: &str = "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞ\u{1b}ÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà";
    let alphabet: Vec<char> = BASE.chars().collect();
    let mut text = String::new();
    let mut escape = false;
    if offset
        .checked_add(count.checked_mul(7).ok_or_else(invalid)?)
        .ok_or_else(invalid)?
        > raw.len() * 8
    {
        return Err(invalid());
    }
    for i in 0..count {
        let bit = offset + i * 7;
        let index = bit / 8;
        let mut value = u16::from(raw[index]) >> (bit % 8);
        if bit % 8 > 1 {
            value |= u16::from(*raw.get(index + 1).ok_or_else(invalid)?) << (8 - bit % 8);
        }
        let septet = (value & 127) as usize;
        if escape {
            let c = match septet {
                10 => '\u{c}',
                20 => '^',
                40 => '{',
                41 => '}',
                47 => '\\',
                60 => '[',
                61 => '~',
                62 => ']',
                64 => '|',
                101 => '€',
                _ => return Err(PanelError::new("SMS_ENCODING_UNSUPPORTED", 422)),
            };
            text.push(c);
            escape = false;
        } else if septet == 27 {
            escape = true;
        } else {
            text.push(*alphabet.get(septet).ok_or_else(invalid)?);
        }
    }
    if escape {
        return Err(invalid());
    }
    Ok(text)
}
fn number(c: &mut Cursor<'_>) -> Result<String> {
    let length = usize::from(c.byte()?);
    let toa = c.byte()?;
    if length == 0 || length > 40 || toa & 0x80 == 0 {
        return Err(invalid());
    }
    let raw = c.take(length.div_ceil(2))?;
    if toa & 0x70 == 0x50 {
        return gsm(raw, length * 4 / 7, 0);
    }
    let mut value = if toa & 0x70 == 0x10 {
        "+".to_owned()
    } else {
        String::new()
    };
    for i in 0..length {
        let digit = if i % 2 == 0 {
            raw[i / 2] & 15
        } else {
            raw[i / 2] >> 4
        };
        if digit > 9 {
            return Err(invalid());
        }
        value.push(char::from(b'0' + digit));
    }
    if length % 2 != 0 && raw[raw.len() - 1] >> 4 != 15 {
        return Err(invalid());
    }
    Ok(value)
}
fn timestamp(raw: &[u8]) -> Result<String> {
    let bcd = |byte: u8| -> Result<u8> {
        if byte & 15 > 9 || byte >> 4 > 9 {
            return Err(invalid());
        }
        Ok((byte & 15) * 10 + (byte >> 4))
    };
    let y = bcd(raw[0])?;
    let m = bcd(raw[1])?;
    let d = bcd(raw[2])?;
    let h = bcd(raw[3])?;
    let minute = bcd(raw[4])?;
    let s = bcd(raw[5])?;
    let tz = bcd(raw[6] & !8)?;
    let year = if y >= 70 {
        1900 + u16::from(y)
    } else {
        2000 + u16::from(y)
    };
    let days = match m {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=12).contains(&m)
        || !(1..=days).contains(&d)
        || h > 23
        || minute > 59
        || s > 59
        || tz > 56
    {
        return Err(invalid());
    }
    Ok(format!(
        "{year:04}-{m:02}-{d:02} {h:02}:{minute:02}:{s:02} {}{:02}:{:02}",
        if raw[6] & 8 != 0 { '-' } else { '+' },
        tz / 4,
        (tz % 4) * 15
    ))
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Concatenation {
    pub reference: u16,
    pub total: u8,
    pub part: u8,
}
#[derive(Clone)]
pub struct Record {
    pub message: Message,
    pub concatenation: Option<Concatenation>,
    pub dcs: u8,
    pub indexes: Vec<u32>,
}
fn user_data(c: &mut Cursor<'_>, dcs: u8, header: bool) -> Result<(String, Option<Concatenation>)> {
    let encoding = if dcs & 0xc0 == 0 {
        if dcs & 0x20 != 0 {
            return Err(PanelError::new("SMS_ENCODING_UNSUPPORTED", 422));
        }
        (dcs >> 2) & 3
    } else {
        match dcs >> 4 {
            12 | 13 => 0,
            14 => 2,
            15 => {
                if dcs & 4 == 0 {
                    0
                } else {
                    1
                }
            }
            _ => return Err(PanelError::new("SMS_ENCODING_UNSUPPORTED", 422)),
        }
    };
    let length = usize::from(c.byte()?);
    let octets = if encoding == 0 {
        (length * 7).div_ceil(8)
    } else {
        length
    };
    if octets > 140 {
        return Err(invalid());
    }
    let raw = c.take(octets)?;
    let mut skip = 0;
    let mut concatenation = None;
    if header {
        skip = usize::from(*raw.first().ok_or_else(invalid)?) + 1;
        if skip > raw.len() {
            return Err(invalid());
        }
        let mut at = 1;
        while at < skip {
            let id = *raw.get(at).ok_or_else(invalid)?;
            let n = usize::from(*raw.get(at + 1).ok_or_else(invalid)?);
            let end = at + 2 + n;
            if end > skip {
                return Err(invalid());
            }
            let data = &raw[at + 2..end];
            let part = match (id, n) {
                (0, 3) => Some(Concatenation {
                    reference: u16::from(data[0]),
                    total: data[1],
                    part: data[2],
                }),
                (8, 4) => Some(Concatenation {
                    reference: u16::from_be_bytes([data[0], data[1]]),
                    total: data[2],
                    part: data[3],
                }),
                (0 | 8, _) => return Err(invalid()),
                (0x24 | 0x25, _) => return Err(PanelError::new("SMS_ENCODING_UNSUPPORTED", 422)),
                _ => None,
            };
            if let Some(part) = part {
                if concatenation.is_some()
                    || part.total < 2
                    || part.total > 32
                    || part.part == 0
                    || part.part > part.total
                {
                    return Err(invalid());
                }
                concatenation = Some(part);
            }
            at = end;
        }
    }
    let text = match encoding {
        0 => {
            let septets = (skip * 8).div_ceil(7);
            if septets > length {
                return Err(invalid());
            }
            gsm(raw, length - septets, septets * 7)?
        }
        1 => return Err(PanelError::new("SMS_BINARY_UNSUPPORTED", 422)),
        2 => {
            let data = &raw[skip..];
            if data.len() % 2 != 0 {
                return Err(invalid());
            }
            char::decode_utf16(
                data.chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]])),
            )
            .collect::<std::result::Result<String, _>>()
            .map_err(|_| invalid())?
        }
        _ => return Err(PanelError::new("SMS_ENCODING_UNSUPPORTED", 422)),
    };
    if c.at != c.raw.len() || text.is_empty() || text.len() > MAX_SMS_BYTES {
        return Err(invalid());
    }
    Ok((text, concatenation))
}
pub fn decode(
    hex: &str,
    tpdu_length: usize,
    modem: &str,
    storage: &str,
    index: u32,
    status: u8,
) -> Result<Record> {
    let raw = bytes(hex)?;
    let mut c = Cursor { raw: &raw, at: 0 };
    let sca = usize::from(c.byte()?);
    c.take(sca)?;
    if raw.len() - c.at != tpdu_length {
        return Err(invalid());
    }
    let first = c.byte()?;
    let mti = first & 3;
    let (contact, timestamp, dcs) = match mti {
        0 => {
            let number = number(&mut c)?;
            c.byte()?;
            let dcs = c.byte()?;
            let time = timestamp(c.take(7)?)?;
            (number, time, dcs)
        }
        1 => {
            c.byte()?;
            let number = number(&mut c)?;
            c.byte()?;
            let dcs = c.byte()?;
            match (first >> 3) & 3 {
                0 => {}
                2 => {
                    c.take(1)?;
                }
                _ => {
                    c.take(7)?;
                }
            }
            (number, String::new(), dcs)
        }
        _ => return Err(PanelError::new("SMS_PDU_TYPE_UNSUPPORTED", 422)),
    };
    let (content, concatenation) = user_data(&mut c, dcs, first & 0x40 != 0)?;
    let direction = match (status, mti) {
        (0 | 1, 0) => Direction::Incoming,
        (2 | 3, 1) => Direction::Outgoing,
        _ => Direction::Unknown,
    };
    let message = Message {
        modem: modem.into(),
        storage: storage.into(),
        index,
        direction,
        contact,
        timestamp,
        content,
    };
    message.validate()?;
    Ok(Record {
        message,
        concatenation,
        dcs,
        indexes: vec![index],
    })
}
pub fn parse_cmgl(raw: &str, modem: &str, storage: &str) -> Result<Vec<Record>> {
    assemble(parse_cmgl_parts(raw, modem, storage)?)
}
pub fn parse_cmgl_parts(raw: &str, modem: &str, storage: &str) -> Result<Vec<Record>> {
    if raw.len() > crate::MAX_AT_RESPONSE {
        return Err(invalid());
    }
    let lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if lines.last() != Some(&"OK") {
        return Err(invalid());
    }
    let mut records = vec![];
    let mut slots = BTreeSet::new();
    let mut at = 0;
    while at + 1 < lines.len() {
        let line = lines[at];
        at += 1;
        if line == "AT+CMGL=4" || crate::sms_notice::has_cmti(line) {
            continue;
        }
        let header = line.strip_prefix("+CMGL:").ok_or_else(invalid)?;
        let first = header.split_once(',').ok_or_else(invalid)?;
        let second = first.1.split_once(',').ok_or_else(invalid)?;
        let last = second.1.rsplit_once(',').ok_or_else(invalid)?;
        let index = first.0.trim().parse::<u32>().map_err(|_| invalid())?;
        let status = second.0.trim().parse::<u8>().map_err(|_| invalid())?;
        let n = last.1.trim().parse::<usize>().map_err(|_| invalid())?;
        if records.len() >= MAX_MESSAGES || index > 65535 || status > 4 || !slots.insert(index) {
            return Err(invalid());
        }
        let pdu = *lines.get(at).ok_or_else(invalid)?;
        at += 1;
        records.push(decode(pdu, n, modem, storage, index, status)?);
    }
    Ok(records)
}
pub fn parse_cmgr(raw: &str, modem: &str, storage: &str, index: u32) -> Result<Record> {
    if raw.len() > crate::MAX_AT_RESPONSE || index > 65535 {
        return Err(invalid());
    }
    let echo = format!("AT+CMGR={index}");
    let mut lines: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if lines.first() == Some(&echo.as_str()) {
        lines.remove(0);
    }
    lines.retain(|line| !crate::sms_notice::has_cmti(line));
    if lines.len() != 3 || lines[2] != "OK" {
        return Err(invalid());
    }
    let header = lines[0].strip_prefix("+CMGR:").ok_or_else(invalid)?;
    let (status, rest) = header.split_once(',').ok_or_else(invalid)?;
    let (alpha, length) = rest.rsplit_once(',').ok_or_else(invalid)?;
    let alpha = alpha.trim();
    if !alpha.is_empty()
        && !(alpha.len() >= 2
            && alpha.starts_with('"')
            && alpha.ends_with('"')
            && !alpha[1..alpha.len() - 1].contains('"'))
    {
        return Err(invalid());
    }
    let status = status.trim().parse::<u8>().map_err(|_| invalid())?;
    if status > 4 {
        return Err(invalid());
    }
    decode(
        lines[1],
        length.trim().parse().map_err(|_| invalid())?,
        modem,
        storage,
        index,
        status,
    )
}
pub fn assemble(records: Vec<Record>) -> Result<Vec<Record>> {
    if records.len() > MAX_MESSAGES {
        return Err(invalid());
    }
    let mut single = vec![];
    type Group = (String, String, Direction, String, String, u16, u8, u8);
    let mut groups: BTreeMap<Group, Vec<Record>> = BTreeMap::new();
    for record in records {
        if let Some(part) = &record.concatenation {
            let time = format!(
                "{}{}",
                record.message.timestamp.get(..10).unwrap_or(""),
                record.message.timestamp.get(20..).unwrap_or("")
            );
            let key = (
                record.message.modem.clone(),
                record.message.storage.clone(),
                record.message.direction,
                record.message.contact.clone(),
                time,
                part.reference,
                part.total,
                record.dcs,
            );
            groups.entry(key).or_default().push(record);
        } else {
            single.push(record);
        }
    }
    for (_, mut parts) in groups {
        let total = usize::from(parts[0].concatenation.as_ref().ok_or_else(invalid)?.total);
        let mut seen = BTreeSet::new();
        if parts.len() > total
            || parts
                .iter()
                .any(|r| !seen.insert(r.concatenation.as_ref().map(|p| p.part)))
        {
            return Err(PanelError::new("SMS_MULTIPART_AMBIGUOUS", 422));
        }
        let times: Vec<u32> = parts
            .iter()
            .filter_map(|r| {
                let t = &r.message.timestamp;
                Some(
                    t.get(11..13)?.parse::<u32>().ok()? * 3600
                        + t.get(14..16)?.parse::<u32>().ok()? * 60
                        + t.get(17..19)?.parse::<u32>().ok()?,
                )
            })
            .collect();
        if let (Some(first), Some(last)) = (times.iter().min(), times.iter().max()) {
            if last - first > 600 {
                return Err(PanelError::new("SMS_MULTIPART_AMBIGUOUS", 422));
            }
        }
        parts.sort_by_key(|r| r.concatenation.as_ref().map(|p| p.part).unwrap_or(0));
        if parts.len() < total {
            single.extend(parts);
            continue;
        }
        for (i, part) in parts.iter().enumerate() {
            if usize::from(part.concatenation.as_ref().ok_or_else(invalid)?.part) != i + 1 {
                return Err(invalid());
            }
        }
        let indexes = parts.iter().flat_map(|r| r.indexes.clone()).collect();
        let content = parts
            .iter()
            .map(|r| r.message.content.as_str())
            .collect::<String>();
        let mut joined = parts.remove(0);
        joined.message.content = content;
        joined.indexes = indexes;
        joined.concatenation = None;
        joined.message.validate()?;
        single.push(joined);
    }
    single.sort_by_key(|r| r.message.index);
    Ok(single)
}
pub struct Submit {
    pub hex: String,
    pub tpdu_bytes: usize,
}
pub fn encode_submit(number: &str, text: &str) -> Result<Submit> {
    let digits = number.strip_prefix('+').unwrap_or(number);
    if digits.is_empty()
        || digits.len() > 20
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || text.is_empty()
        || text.chars().any(|c| c == '\0')
    {
        return Err(PanelError::new("SMS_SEND_INVALID", 422));
    }
    let data: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
    if data.len() > 140 {
        return Err(PanelError::new("SMS_SEND_TOO_LONG", 422));
    }
    let mut raw = vec![
        0,
        1,
        0,
        digits.len() as u8,
        if number.starts_with('+') { 0x91 } else { 0x81 },
    ];
    let digits = digits.as_bytes();
    for pair in digits.chunks(2) {
        raw.push((pair[0] - b'0') | ((pair.get(1).map(|b| b - b'0').unwrap_or(15)) << 4));
    }
    raw.extend_from_slice(&[0, 8, data.len() as u8]);
    raw.extend_from_slice(&data);
    Ok(Submit {
        tpdu_bytes: raw.len() - 1,
        hex: raw.iter().map(|b| format!("{b:02X}")).collect(),
    })
}
