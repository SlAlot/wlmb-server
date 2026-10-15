//! 有界ubus二进制消息编解码，校验容器深度、长度和字段类型。
use crate::{PanelError, Result};
use serde_json::{Map, Value, json};

pub const LIMIT: usize = 65_536;
fn invalid() -> PanelError {
    PanelError::new("UBUS_FORMAT", 503)
}
fn padded(n: usize) -> usize {
    (n + 3) & !3
}
pub fn attr(id: u8, extended: bool, data: &[u8]) -> Result<Vec<u8>> {
    let n = 4 + data.len();
    if id > 127 || n > LIMIT {
        return Err(invalid());
    }
    let flags = (u32::from(id) << 24) | if extended { 1 << 31 } else { 0 } | n as u32;
    let mut out = flags.to_be_bytes().to_vec();
    out.extend_from_slice(data);
    out.resize(padded(n), 0);
    Ok(out)
}
pub fn attrs(mut raw: &[u8]) -> Result<Vec<(u8, bool, &[u8])>> {
    if raw.len() > LIMIT {
        return Err(invalid());
    }
    let mut rows = vec![];
    while !raw.is_empty() {
        if rows.len() >= 4096 || raw.len() < 4 {
            return Err(invalid());
        }
        let header = u32::from_be_bytes(raw[..4].try_into().map_err(|_| invalid())?);
        let size = (header & 0x00ff_ffff) as usize;
        if size < 4 || padded(size) > raw.len() {
            return Err(invalid());
        }
        rows.push((
            ((header >> 24) & 127) as u8,
            header >> 31 != 0,
            &raw[4..size],
        ));
        raw = &raw[padded(size)..];
    }
    Ok(rows)
}
fn string(raw: &[u8]) -> Result<&str> {
    let s = raw.strip_suffix(&[0]).ok_or_else(invalid)?;
    if s.contains(&0) {
        return Err(invalid());
    }
    std::str::from_utf8(s).map_err(|_| invalid())
}
fn encode_node(name: &str, value: &Value, depth: usize, nodes: &mut usize) -> Result<Vec<u8>> {
    *nodes += 1;
    if depth > 24 || *nodes > 4096 || name.len() > 255 || name.contains('\0') {
        return Err(invalid());
    }
    let (kind, body) = match value {
        Value::Null => (0, vec![]),
        Value::Bool(v) => (7, vec![u8::from(*v)]),
        Value::String(v) if !v.contains('\0') => {
            let mut b = v.as_bytes().to_vec();
            b.push(0);
            (3, b)
        }
        Value::Number(v) => {
            if let Some(n) = v.as_i64() {
                if let Ok(n) = i32::try_from(n) {
                    (5, n.to_be_bytes().to_vec())
                } else {
                    (4, n.to_be_bytes().to_vec())
                }
            } else if let Some(n) = v.as_f64().filter(|n| n.is_finite()) {
                (8, n.to_bits().to_be_bytes().to_vec())
            } else {
                return Err(invalid());
            }
        }
        Value::Array(values) => {
            let mut b = vec![];
            for v in values {
                b.extend(encode_node("", v, depth + 1, nodes)?);
                if b.len() > LIMIT {
                    return Err(invalid());
                }
            }
            (1, b)
        }
        Value::Object(values) => {
            let mut b = vec![];
            for (k, v) in values {
                b.extend(encode_node(k, v, depth + 1, nodes)?);
                if b.len() > LIMIT {
                    return Err(invalid());
                }
            }
            (2, b)
        }
        _ => return Err(invalid()),
    };
    let mut data = (name.len() as u16).to_be_bytes().to_vec();
    data.extend_from_slice(name.as_bytes());
    data.push(0);
    data.resize(padded(data.len()), 0);
    data.extend(body);
    attr(kind, true, &data)
}
pub fn encode_object(value: &Value) -> Result<Vec<u8>> {
    let map = value.as_object().ok_or_else(invalid)?;
    let mut out = vec![];
    let mut count = 0;
    for (name, value) in map {
        out.extend(encode_node(name, value, 0, &mut count)?);
        if out.len() > LIMIT {
            return Err(invalid());
        }
    }
    Ok(out)
}
fn decode_nodes(raw: &[u8], depth: usize, count: &mut usize, array: bool) -> Result<Value> {
    if depth > 24 {
        return Err(invalid());
    }
    let mut map = Map::new();
    let mut list = vec![];
    for (kind, extended, raw) in attrs(raw)? {
        *count += 1;
        if *count > 4096 || !extended || raw.len() < 4 {
            return Err(invalid());
        }
        let size = u16::from_be_bytes([raw[0], raw[1]]) as usize;
        let header = padded(3 + size);
        if size > 255 || header > raw.len() {
            return Err(invalid());
        }
        let name = string(&raw[2..3 + size])?;
        if array && !name.is_empty() || !array && map.contains_key(name) {
            return Err(invalid());
        }
        let data = &raw[header..];
        let value = match (kind, data.len()) {
            (0, 0) => Value::Null,
            (1, _) => decode_nodes(data, depth + 1, count, true)?,
            (2, _) => decode_nodes(data, depth + 1, count, false)?,
            (3, _) => json!(string(data)?),
            (4, 8) => json!(i64::from_be_bytes(data.try_into().map_err(|_| invalid())?)),
            (5, 4) => json!(i32::from_be_bytes(data.try_into().map_err(|_| invalid())?)),
            (6, 2) => json!(i16::from_be_bytes(data.try_into().map_err(|_| invalid())?)),
            (7, 1) if data[0] <= 1 => json!(data[0] != 0),
            (8, 8) => {
                let n = f64::from_bits(u64::from_be_bytes(data.try_into().map_err(|_| invalid())?));
                if !n.is_finite() {
                    return Err(invalid());
                }
                json!(n)
            }
            _ => return Err(invalid()),
        };
        if array {
            list.push(value);
        } else {
            map.insert(name.to_owned(), value);
        }
    }
    Ok(if array {
        Value::Array(list)
    } else {
        Value::Object(map)
    })
}
pub fn decode_object(raw: &[u8]) -> Result<Value> {
    decode_nodes(raw, 0, &mut 0, false)
}
