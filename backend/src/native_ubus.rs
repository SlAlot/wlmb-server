//! 独立实现的ubus线协议；只允许固定对象和方法，消息长度与等待时间均受限。
use crate::{PanelError, Result, blob, transport::Bus};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    sync::Mutex,
    time::{Duration, Instant},
};
fn fail() -> PanelError {
    PanelError::new("SYSTEM_UNAVAILABLE", 503)
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|v| !v.is_zero())
        .ok_or_else(fail)
}
fn exact(stream: &mut UnixStream, raw: &mut [u8], deadline: Instant) -> Result<()> {
    let mut offset = 0;
    while offset < raw.len() {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|_| fail())?;
        let n = stream.read(&mut raw[offset..]).map_err(|_| fail())?;
        if n == 0 {
            return Err(fail());
        }
        offset += n;
    }
    Ok(())
}
fn frame(stream: &mut UnixStream, deadline: Instant) -> Result<(u8, u16, u32, Vec<u8>)> {
    let mut head = [0; 12];
    exact(stream, &mut head, deadline)?;
    let len =
        (u32::from_be_bytes(head[8..12].try_into().map_err(|_| fail())?) & 0x00ff_ffff) as usize;
    if head[0] != 0 || !(4..=blob::LIMIT).contains(&len) || len % 4 != 0 || head[8] != 0 {
        return Err(fail());
    }
    let mut raw = vec![0; len - 4];
    exact(stream, &mut raw, deadline)?;
    Ok((
        head[1],
        u16::from_be_bytes([head[2], head[3]]),
        u32::from_be_bytes(head[4..8].try_into().map_err(|_| fail())?),
        raw,
    ))
}
fn u32_attr(raw: &[u8]) -> Result<u32> {
    Ok(u32::from_be_bytes(raw.try_into().map_err(|_| fail())?))
}
struct Channel {
    socket: UnixStream,
    sequence: u16,
    events: Vec<Vec<u8>>,
}
type Replies = Vec<Vec<(u8, Vec<u8>)>>;
impl Channel {
    fn connect(deadline: Instant) -> Result<Self> {
        let mut socket = UnixStream::connect("/var/run/ubus/ubus.sock").map_err(|_| fail())?;
        let (kind, _, peer, _) = frame(&mut socket, deadline)?;
        if kind != 0 || peer <= 1 {
            return Err(fail());
        }
        Ok(Self {
            socket,
            sequence: 0,
            events: vec![],
        })
    }
    fn request(
        &mut self,
        kind: u8,
        peer: u32,
        attrs: Vec<u8>,
        deadline: Instant,
    ) -> Result<Replies> {
        self.sequence = self.sequence.wrapping_add(1);
        let seq = self.sequence;
        let mut request = vec![0, kind];
        request.extend(seq.to_be_bytes());
        request.extend(peer.to_be_bytes());
        request.extend(blob::attr(0, false, &attrs)?);
        let mut offset = 0;
        while offset < request.len() {
            self.socket
                .set_write_timeout(Some(remaining(deadline)?))
                .map_err(|_| fail())?;
            let n = self.socket.write(&request[offset..]).map_err(|_| fail())?;
            if n == 0 {
                return Err(fail());
            }
            offset += n;
        }
        let mut replies = vec![];
        let request_kind = kind;
        let mut total = 0;
        for _ in 0..16 {
            let (kind, answer_seq, answer_peer, raw) = frame(&mut self.socket, deadline)?;
            if kind == 5 && answer_peer == 0 && self.events.len() < 16 {
                self.events.push(raw);
                continue;
            }
            if answer_seq != seq || answer_peer != peer {
                return Err(fail());
            }
            total += raw.len();
            if total > 262144 {
                return Err(fail());
            }
            let attrs = blob::attrs(&raw)?;
            if attrs.iter().any(|(_, ext, _)| *ext) {
                return Err(fail());
            }
            if kind == 1 {
                let status = attrs
                    .iter()
                    .filter(|(id, _, _)| *id == 1)
                    .collect::<Vec<_>>();
                if status.len() == 1 && request_kind == 4 && u32_attr(status[0].2)? == 4 {
                    return Err(PanelError::new("UBUS_OBJECT_NOT_FOUND", 503));
                }
                if status.len() != 1 || u32_attr(status[0].2)? != 0 {
                    return Err(fail());
                }
                return Ok(replies);
            }
            if kind != 2 {
                return Err(fail());
            }
            replies.push(
                attrs
                    .into_iter()
                    .map(|(id, _, data)| (id, data.to_vec()))
                    .collect(),
            );
        }
        Err(fail())
    }
    fn call(
        &mut self,
        object: &str,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<Value> {
        let mut path = object.as_bytes().to_vec();
        path.push(0);
        let lookup = self.request(4, 0, blob::attr(2, false, &path)?, deadline)?;
        if lookup.len() != 1 {
            return Err(fail());
        }
        let ids = lookup[0]
            .iter()
            .filter(|(id, _)| *id == 3)
            .collect::<Vec<_>>();
        let paths = lookup[0]
            .iter()
            .filter(|(id, _)| *id == 2)
            .collect::<Vec<_>>();
        if ids.len() != 1 || paths.len() != 1 || paths[0].1 != path {
            return Err(fail());
        }
        let id = u32_attr(&ids[0].1)?;
        let mut method = method.as_bytes().to_vec();
        method.push(0);
        let mut args = blob::attr(3, false, &id.to_be_bytes())?;
        args.extend(blob::attr(4, false, &method)?);
        args.extend(blob::attr(7, false, &blob::encode_object(&params)?)?);
        let replies = self.request(5, id, args, deadline)?;
        if replies.is_empty() {
            return Ok(json!({}));
        }
        if replies.len() != 1 {
            return Err(fail());
        }
        let data = replies[0]
            .iter()
            .filter(|(id, _)| *id == 7)
            .collect::<Vec<_>>();
        if data.len() != 1 {
            return Err(fail());
        }
        blob::decode_object(&data[0].1)
    }
}
struct SmsEventChannel {
    pattern: &'static str,
    channel: Channel,
    object: u32,
    buffer: Vec<u8>,
}
impl SmsEventChannel {
    fn connect(pattern: &'static str) -> Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut channel = Channel::connect(deadline)?;
        let replies = channel.request(6, 0, vec![], deadline)?;
        let ids: Vec<_> = replies
            .iter()
            .flatten()
            .filter(|(id, _)| *id == 3)
            .collect();
        if ids.len() != 1 {
            return Err(fail());
        }
        let object = u32_attr(&ids[0].1)?;
        if object < 1024 {
            return Err(fail());
        }
        let mut attrs = blob::attr(3, false, &1u32.to_be_bytes())?;
        attrs.extend(blob::attr(4, false, b"register\0")?);
        attrs.extend(blob::attr(
            7,
            false,
            &blob::encode_object(&json!({"object":object as i32,"pattern":pattern}))?,
        )?);
        channel.request(5, 1, attrs, deadline)?;
        Ok(Self {
            pattern,
            channel,
            object,
            buffer: vec![],
        })
    }
    fn event(&self, raw: &[u8]) -> Result<Value> {
        let attrs = blob::attrs(raw)?;
        let field = |key| {
            let matches: Vec<_> = attrs.iter().filter(|(id, _, _)| *id == key).collect();
            if matches.len() != 1 || matches[0].1 {
                return Err(fail());
            }
            Ok(matches[0].2)
        };
        if u32_attr(field(3)?)? != self.object
            || field(4)?.strip_suffix(&[0]) != Some(self.pattern.as_bytes())
            || field(10)? != [1]
        {
            return Err(fail());
        }
        blob::decode_object(field(7)?)
    }
}
impl crate::transport::SmsEvents for SmsEventChannel {
    fn next(&mut self, timeout: Duration) -> Result<Option<Value>> {
        if timeout.is_zero() || timeout > Duration::from_secs(1) {
            return Err(fail());
        }
        if !self.channel.events.is_empty() {
            let raw = self.channel.events.remove(0);
            return self.event(&raw).map(Some);
        }
        let deadline = Instant::now() + timeout;
        loop {
            if self.buffer.len() >= 12 {
                let head = &self.buffer[..12];
                let len = (u32::from_be_bytes(head[8..12].try_into().map_err(|_| fail())?)
                    & 0x00ff_ffff) as usize;
                if head[0] != 0
                    || head[1] != 5
                    || head[4..8] != [0, 0, 0, 0]
                    || head[8] != 0
                    || !(4..=blob::LIMIT).contains(&len)
                    || len % 4 != 0
                {
                    return Err(fail());
                }
                let total = 8 + len;
                if self.buffer.len() >= total {
                    let result = self.event(&self.buffer[12..total]);
                    self.buffer.drain(..total);
                    return result.map(Some);
                }
            }
            let Some(left) = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
            else {
                return Ok(None);
            };
            self.channel
                .socket
                .set_read_timeout(Some(left))
                .map_err(|_| fail())?;
            let mut bytes = [0; 4096];
            match self.channel.socket.read(&mut bytes) {
                Ok(0) => return Err(fail()),
                Ok(n) => self.buffer.extend(&bytes[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(_) => return Err(fail()),
            }
            if self.buffer.len() > blob::LIMIT + 4096 {
                return Err(fail());
            }
        }
    }
}
#[derive(Default)]
struct State {
    channel: Option<Channel>,
    closed: bool,
}
#[derive(Default)]
pub struct NativeBus {
    state: Mutex<State>,
}
impl NativeBus {
    pub fn close(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| fail())?;
        state.closed = true;
        state.channel.take();
        Ok(())
    }
}
impl Bus for NativeBus {
    fn sms_events(&self) -> Result<Box<dyn crate::transport::SmsEvents>> {
        SmsEventChannel::connect("qmodem.at.line")
            .map(|v| Box::new(v) as Box<dyn crate::transport::SmsEvents>)
    }
    fn qmodem_sms_events(&self) -> Result<Box<dyn crate::transport::SmsEvents>> {
        SmsEventChannel::connect("qmodem.at.urc")
            .map(|v| Box::new(v) as Box<dyn crate::transport::SmsEvents>)
    }
    fn describe(&self, object: &str, timeout: Duration) -> Result<Value> {
        if !["at-daemon", "qmodem.sms"].contains(&object)
            || timeout.is_zero()
            || timeout > Duration::from_secs(5)
        {
            return Err(fail());
        }
        let deadline = Instant::now() + timeout;
        let mut state = loop {
            match self.state.try_lock() {
                Ok(v) => break v,
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(fail()),
                Err(_) => {
                    remaining(deadline)?;
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        };
        if state.closed {
            return Err(fail());
        }
        if state.channel.is_none() {
            state.channel = Some(Channel::connect(deadline)?);
        }
        let result = (|| {
            let mut path = object.as_bytes().to_vec();
            path.push(0);
            let replies = state.channel.as_mut().ok_or_else(fail)?.request(
                4,
                0,
                blob::attr(2, false, &path)?,
                deadline,
            )?;
            if replies.len() != 1 {
                if replies.is_empty() {
                    return Err(PanelError::new("UBUS_OBJECT_NOT_FOUND", 503));
                }
                return Err(fail());
            }
            let paths: Vec<_> = replies[0].iter().filter(|(id, _)| *id == 2).collect();
            let signatures: Vec<_> = replies[0].iter().filter(|(id, _)| *id == 6).collect();
            if paths.len() != 1 || paths[0].1 != path || signatures.len() != 1 {
                return Err(fail());
            }
            blob::decode_object(&signatures[0].1)
        })();
        if result.is_err() {
            state.channel.take();
        }
        result
    }
    fn call(&self, object: &str, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        if timeout.is_zero()
            || timeout > Duration::from_secs(30)
            || !params.is_object()
            || !crate::transport::allowed(object, method)
        {
            return Err(fail());
        }
        let deadline = Instant::now() + timeout;
        let mut state = loop {
            match self.state.try_lock() {
                Ok(v) => break v,
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(fail()),
                Err(_) => {
                    remaining(deadline)?;
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        };
        if state.closed {
            return Err(fail());
        }
        if state.channel.is_none() {
            state.channel = Some(Channel::connect(deadline)?);
        }
        let result = state
            .channel
            .as_mut()
            .ok_or_else(fail)?
            .call(object, method, params, deadline);
        if result.is_err() {
            state.channel.take();
        }
        result
    }
}

#[cfg(test)]
mod event_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::transport::SmsEvents;
    fn attrs(object: u32, method: &[u8], value: Value) -> Vec<u8> {
        let mut raw = blob::attr(3, false, &object.to_be_bytes()).unwrap();
        raw.extend(blob::attr(4, false, method).unwrap());
        raw.extend(blob::attr(10, false, &[1]).unwrap());
        raw.extend(blob::attr(7, false, &blob::encode_object(&value).unwrap()).unwrap());
        raw
    }
    fn connection() -> (SmsEventChannel, UnixStream) {
        let (socket, peer) = UnixStream::pair().unwrap();
        (
            SmsEventChannel {
                pattern: "qmodem.at.line",
                channel: Channel {
                    socket,
                    sequence: 0,
                    events: vec![],
                },
                object: 1234,
                buffer: vec![],
            },
            peer,
        )
    }
    #[test]
    fn event_frames_require_exact_registered_object_method_and_no_reply() {
        let (c, _) = connection();
        let value = json!({"port":"/dev/ttyUSB1","raw_line":"+CMTI: \"ME\",1"});
        assert_eq!(
            c.event(&attrs(1234, b"qmodem.at.line\0", value.clone()))
                .unwrap(),
            value
        );
        assert!(
            c.event(&attrs(1235, b"qmodem.at.line\0", value.clone()))
                .is_err()
        );
        assert!(
            c.event(&attrs(1234, b"qmodem.at.line-other\0", value.clone()))
                .is_err()
        );
        let mut duplicate = attrs(1234, b"qmodem.at.line\0", value);
        duplicate.extend(blob::attr(3, false, &1234u32.to_be_bytes()).unwrap());
        assert!(c.event(&duplicate).is_err());
    }
    #[test]
    fn database_urc_channel_never_accepts_raw_line_events() {
        let (mut channel, _) = connection();
        channel.pattern = "qmodem.at.urc";
        let event = json!({"port":"/dev/ttyUSB2","owner":"qmodem.sms","urc_id":"sms-new","restart_epoch":1,"sequence":1});
        assert_eq!(
            channel
                .event(&attrs(1234, b"qmodem.at.urc\0", event.clone()))
                .unwrap(),
            event
        );
        assert!(
            channel
                .event(&attrs(1234, b"qmodem.at.line\0", event))
                .is_err()
        );
    }
    #[test]
    fn native_event_read_preserves_fragmented_frames_and_idle_timeout_without_reconnecting() {
        let (mut c, mut peer) = connection();
        let value = json!({"port":"/dev/ttyUSB1","correlation":"idle","restart_epoch":1,"sequence":1,"drop_count":0,"raw_line":"+CMTI: \"ME\",1"});
        let mut raw = vec![0, 5, 0, 1, 0, 0, 0, 0];
        raw.extend(blob::attr(0, false, &attrs(1234, b"qmodem.at.line\0", value.clone())).unwrap());
        peer.write_all(&raw[..10]).unwrap();
        assert!(c.next(Duration::from_millis(5)).unwrap().is_none());
        assert_eq!(c.buffer.len(), 10);
        peer.write_all(&raw[10..]).unwrap();
        assert_eq!(c.next(Duration::from_millis(20)).unwrap(), Some(value));
        assert!(c.next(Duration::from_millis(5)).unwrap().is_none());
        assert!(c.buffer.is_empty());
    }
}
