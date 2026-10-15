//! 独立串口的命令／提示符事务、空闲通知与异常隔离，防止回复串线。
use crate::{MAX_AT_RESPONSE, PanelError, Result, security};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};

pub trait Wire: Read + Write + Send {
    fn timeout(&mut self, duration: Duration) -> Result<()>;
}
fn error(code: &'static str) -> PanelError {
    PanelError::new(code, 503)
}
pub struct Channel<W: Wire> {
    wire: W,
    poisoned: bool,
    notice_line: Vec<u8>,
    notices: u64,
    sms_hints: crate::sms_notice::Hints,
}
impl<W: Wire> Channel<W> {
    pub fn new(wire: W) -> Self {
        Self {
            wire,
            poisoned: false,
            notice_line: Vec::new(),
            notices: 0,
            sms_hints: crate::sms_notice::Hints::default(),
        }
    }
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }
    pub fn take_notices(&mut self) -> u64 {
        std::mem::take(&mut self.notices)
    }
    pub fn take_sms_hints(&mut self) -> crate::sms_notice::Hints {
        std::mem::take(&mut self.sms_hints)
    }
    fn notice_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        for byte in bytes.iter().copied() {
            if matches!(byte, b'\r' | b'\n') {
                let line = std::str::from_utf8(&self.notice_line)
                    .map_err(|_| error("AT_RESPONSE_INVALID"))?
                    .trim();
                if matches!(line, "OK" | "ERROR")
                    || line.starts_with("+CME ERROR:")
                    || line.starts_with("+CMS ERROR:")
                    || line.starts_with("+CMGS:")
                    || line.starts_with("+CMGL:")
                    || line.starts_with("+CMGR:")
                {
                    return Err(error("AT_FRAME_AMBIGUOUS"));
                }
                if !line.is_empty()
                    && !line.starts_with('+')
                    && !line.starts_with('^')
                    && !["RING", "NO CARRIER"].contains(&line)
                {
                    return Err(error("AT_FRAME_AMBIGUOUS"));
                }
                if line.starts_with("+CMTI:") && !crate::sms_notice::has_cmti(line) {
                    return Err(error("AT_FRAME_AMBIGUOUS"));
                }
                if crate::sms_notice::has_cmti(line) {
                    self.notices = self.notices.saturating_add(1);
                    self.sms_hints.observe(line);
                }
                self.notice_line.clear();
            } else {
                if self.notice_line.len() >= 512 {
                    return Err(error("AT_RESPONSE_TOO_LARGE"));
                }
                self.notice_line.push(byte);
            }
        }
        Ok(())
    }
    pub fn poll_notices(&mut self, timeout: Duration) -> Result<()> {
        if timeout.is_zero() || timeout > Duration::from_millis(100) {
            return Err(error("AT_INVALID_TIMEOUT"));
        }
        self.transaction(|this| {
            let deadline = Instant::now() + timeout;
            let mut bytes = [0; 1024];
            let mut total = 0;
            while let Some(remaining) = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
            {
                this.wire.timeout(remaining)?;
                match this.wire.read(&mut bytes) {
                    Ok(0) => return Err(error("AT_DISCONNECTED")),
                    Ok(n) => {
                        total += n;
                        if total > 4096 {
                            return Err(error("AT_RESPONSE_TOO_LARGE"));
                        }
                        this.notice_bytes(&bytes[..n])?;
                    }
                    Err(e)
                        if e.kind() == std::io::ErrorKind::TimedOut
                            || e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => return Err(error("AT_CHANNEL_UNAVAILABLE")),
                }
            }
            Ok(())
        })
    }
    fn finish_notice(&mut self) -> Result<()> {
        if !self.notice_line.is_empty() {
            self.poll_notices(Duration::from_millis(100))?;
            if !self.notice_line.is_empty() {
                return Err(error("AT_FRAME_AMBIGUOUS"));
            }
        }
        Ok(())
    }
    pub fn initial_quiet(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut total = 0;
        let mut bytes = [0; 1024];
        self.transaction(|this| {
            loop {
                this.wire
                    .timeout(Self::remaining(deadline)?.min(Duration::from_millis(200)))?;
                match this.wire.read(&mut bytes) {
                    Ok(0) => return Err(error("AT_DISCONNECTED")),
                    Ok(n) => {
                        total += n;
                        if total > MAX_AT_RESPONSE {
                            return Err(error("AT_RESPONSE_TOO_LARGE"));
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => return Err(error("AT_CHANNEL_UNAVAILABLE")),
                }
            }
        })
    }
    fn remaining(deadline: Instant) -> Result<Duration> {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| error("AT_TIMEOUT"))
    }
    fn send(&mut self, raw: &[u8], deadline: Instant) -> Result<()> {
        let mut sent = 0;
        while sent < raw.len() {
            self.wire.timeout(Self::remaining(deadline)?)?;
            let count = self
                .wire
                .write(&raw[sent..])
                .map_err(|_| error("AT_WRITE_UNKNOWN"))?;
            if count == 0 {
                return Err(error("AT_WRITE_UNKNOWN"));
            }
            sent += count;
        }
        Ok(())
    }
    fn receive(&mut self, deadline: Instant, prompt: bool) -> Result<String> {
        let mut raw = Vec::new();
        let mut line: Vec<u8> = Vec::new();
        let mut bytes = [0; 1024];
        loop {
            self.wire.timeout(Self::remaining(deadline)?)?;
            let count = self
                .wire
                .read(&mut bytes)
                .map_err(|_| error("AT_TIMEOUT"))?;
            if count == 0 {
                return Err(error("AT_DISCONNECTED"));
            }
            for (index, byte) in bytes[..count].iter().copied().enumerate() {
                if raw.len() >= MAX_AT_RESPONSE {
                    return Err(error("AT_RESPONSE_TOO_LARGE"));
                }
                raw.push(byte);
                if prompt && byte == b'>' && line.iter().all(|b| b.is_ascii_whitespace()) {
                    if bytes[index + 1..count]
                        .iter()
                        .any(|b| !b.is_ascii_whitespace())
                    {
                        return Err(error("AT_FRAME_AMBIGUOUS"));
                    }
                    return String::from_utf8(raw).map_err(|_| error("AT_RESPONSE_INVALID"));
                }
                if matches!(byte, b'\r' | b'\n') {
                    let text = std::str::from_utf8(&line)
                        .map_err(|_| error("AT_RESPONSE_INVALID"))?
                        .trim();
                    if matches!(text, "OK" | "ERROR")
                        || text.starts_with("+CME ERROR:")
                        || text.starts_with("+CMS ERROR:")
                    {
                        let tail = std::str::from_utf8(&bytes[index + 1..count])
                            .map_err(|_| error("AT_RESPONSE_INVALID"))?;
                        self.notice_bytes(tail.as_bytes())?;
                        if text != "OK" {
                            return Err(PanelError::new("AT_UNSUPPORTED", 422));
                        }
                        if prompt {
                            return Err(error("SMS_PROMPT_MISSING"));
                        }
                        return String::from_utf8(raw).map_err(|_| error("AT_RESPONSE_INVALID"));
                    }
                    line.clear();
                } else {
                    line.push(byte);
                }
            }
        }
    }
    fn transaction<T>(&mut self, action: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.poisoned {
            return Err(error("AT_CHANNEL_QUARANTINED"));
        }
        let result = action(self);
        if result.as_ref().is_err_and(|e| e.code != "AT_UNSUPPORTED") {
            self.poisoned = true;
        }
        result
    }
    pub fn query(&mut self, command: &str, timeout: Duration) -> Result<String> {
        security::validate_at(command)?;
        if timeout.is_zero() || timeout > Duration::from_secs(10) {
            return Err(error("AT_INVALID_TIMEOUT"));
        }
        self.transaction(|this| {
            this.finish_notice()?;
            let deadline = Instant::now() + timeout;
            this.send(format!("{command}\r").as_bytes(), deadline)?;
            let raw = this.receive(deadline, false)?;
            // 短信正文可能伪装成通知；仅从元数据回复或空闲串口接收索引。
            if crate::sms_notice::metadata_or_non_sms(command) && crate::sms_notice::has_cmti(&raw)
            {
                this.sms_hints.observe(&raw);
                this.notices = this.notices.saturating_add(1);
            }
            Ok(raw)
        })
    }
    pub fn send_pdu(&mut self, pdu: &str, tpdu_bytes: usize) -> Result<String> {
        if pdu.is_empty()
            || pdu.len() > 512
            || pdu.len() % 2 != 0
            || !pdu.bytes().all(|b| b.is_ascii_hexdigit())
            || tpdu_bytes == 0
            || tpdu_bytes > 255
        {
            return Err(PanelError::new("SMS_INVALID_PDU", 422));
        }
        let sca = usize::from_str_radix(&pdu[..2], 16)
            .map_err(|_| PanelError::new("SMS_INVALID_PDU", 422))?;
        if pdu.len() / 2 != 1 + sca + tpdu_bytes {
            return Err(PanelError::new("SMS_INVALID_PDU", 422));
        }
        self.transaction(|this| {
            this.finish_notice()?;
            let deadline = Instant::now() + Duration::from_secs(30);
            this.send(format!("AT+CMGS={tpdu_bytes}\r").as_bytes(), deadline)?;
            this.receive(deadline, true)?;
            let mut payload = pdu.as_bytes().to_vec();
            payload.push(0x1a);
            this.send(&payload, deadline)?;
            let result = this.receive(deadline, false)?;
            crate::sms_session::send_reference_with_pdu_echo(&result, pdu)?;
            Ok(result)
        })
    }
}

#[cfg(unix)]
pub mod native {
    use super::*;
    use serialport::SerialPort;
    use std::{
        fs,
        os::{
            fd::AsRawFd,
            unix::fs::{FileTypeExt, MetadataExt},
        },
    };
    impl Wire for serialport::TTYPort {
        fn timeout(&mut self, duration: Duration) -> Result<()> {
            self.set_timeout(duration)
                .map_err(|_| error("AT_CHANNEL_UNAVAILABLE"))
        }
    }
    pub fn open(port: &str) -> Result<Channel<serialport::TTYPort>> {
        if !crate::modem_inventory::valid_port(port) {
            return Err(error("AT_INVALID_PORT"));
        }
        let before = fs::symlink_metadata(port).map_err(|_| error("AT_CHANNEL_UNAVAILABLE"))?;
        if !before.file_type().is_char_device() {
            return Err(error("AT_INVALID_PORT"));
        }
        let tty = serialport::new(port, 115200)
            .preserve_dtr_on_open()
            .timeout(Duration::from_secs(3))
            .open_native()
            .map_err(|_| error("AT_PORT_CONFLICT"))?;
        if !tty.exclusive() {
            return Err(error("AT_PORT_CONFLICT"));
        }
        let descriptor = fs::metadata(format!("/proc/self/fd/{}", tty.as_raw_fd()))
            .map_err(|_| error("AT_OWNERSHIP_UNKNOWN"))?;
        let after = fs::symlink_metadata(port).map_err(|_| error("AT_OWNERSHIP_UNKNOWN"))?;
        if !after.file_type().is_char_device()
            || before.rdev() != after.rdev()
            || before.ino() != after.ino()
            || descriptor.rdev() != before.rdev()
        {
            return Err(error("MODEM_CHANGED"));
        }
        let mut mode = nix::sys::termios::tcgetattr(tty.as_raw_fd())
            .map_err(|_| error("AT_CHANNEL_UNAVAILABLE"))?;
        mode.control_flags
            .remove(nix::sys::termios::ControlFlags::HUPCL);
        nix::sys::termios::tcsetattr(tty.as_raw_fd(), nix::sys::termios::SetArg::TCSANOW, &mode)
            .map_err(|_| error("AT_CHANNEL_UNAVAILABLE"))?;
        Ok(Channel::new(tty))
    }
}
