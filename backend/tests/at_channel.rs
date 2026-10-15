#![allow(clippy::unwrap_used)]
// 离线回归用例；仅使用模拟通道或临时测试资源。
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
use wlmb_server::{
    Result,
    at_channel::{Channel, Wire},
};
struct Fake {
    incoming: VecDeque<Vec<u8>>,
    written: Arc<Mutex<Vec<u8>>>,
}
impl Read for Fake {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let Some(mut raw) = self.incoming.pop_front() else {
            return Err(io::ErrorKind::TimedOut.into());
        };
        let n = raw.len().min(out.len());
        out[..n].copy_from_slice(&raw[..n]);
        if n < raw.len() {
            raw.drain(..n);
            self.incoming.push_front(raw);
        }
        Ok(n)
    }
}
impl Write for Fake {
    fn write(&mut self, raw: &[u8]) -> io::Result<usize> {
        let n = raw.len().min(3);
        self.written.lock().unwrap().extend_from_slice(&raw[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("never block on tcdrain")
    }
}
impl Wire for Fake {
    fn timeout(&mut self, _: Duration) -> Result<()> {
        Ok(())
    }
}
fn fake(chunks: &[&str]) -> (Channel<Fake>, Arc<Mutex<Vec<u8>>>) {
    let written = Arc::new(Mutex::new(vec![]));
    (
        Channel::new(Fake {
            incoming: chunks.iter().map(|c| c.as_bytes().to_vec()).collect(),
            written: written.clone(),
        }),
        written,
    )
}
#[test]
fn sms_index_hints_cover_idle_tail_and_metadata_but_never_sms_body() {
    let (mut channel, written) = fake(&["+CMTI: \"ME\",2\n"]);
    channel.poll_notices(Duration::from_millis(10)).unwrap();
    assert!(channel.take_sms_hints().indexes.contains(&("ME".into(), 2)));
    assert!(written.lock().unwrap().is_empty());
    let (mut channel, _) = fake(&["+CMGF:0\n+CMTI: \"ME\",3\nOK\n+CMTI: \"ME\",4\n"]);
    channel.query("AT+CMGF?", Duration::from_secs(1)).unwrap();
    assert_eq!(channel.take_sms_hints().indexes.len(), 2);
    assert!(channel.take_sms_hints().indexes.is_empty());
    let (mut channel, _) = fake(&["+CMGR: body\n+CMTI: \"ME\",99\nOK\n"]);
    channel.query("AT+CMGR=1", Duration::from_secs(1)).unwrap();
    assert!(channel.take_sms_hints().indexes.is_empty());
    assert_eq!(channel.take_notices(), 0);
}
#[test]
fn initial_buffer_is_drained_without_sending_or_recovering_poisoned_stream() {
    let (mut channel, written) = fake(&["old reply\r\nOK\r\n", "+CMTI: \"ME\",1\r\n"]);
    channel.initial_quiet().unwrap();
    assert!(written.lock().unwrap().is_empty());
    assert!(!channel.poisoned());
    assert!(channel.query("AT", Duration::from_secs(1)).is_err());
    assert_eq!(
        channel.initial_quiet().err().unwrap().code,
        "AT_CHANNEL_QUARANTINED"
    );
}
#[test]
fn fragmented_commands_echo_urc_and_result_are_bounded() {
    let (mut channel, written) = fake(&[
        "AT^MONSC\r\n^MON",
        "SC: NR,78\r\n",
        "\r\nO",
        "K\r\n+CMTI: \"ME\",1\r\n",
    ]);
    let raw = channel.query("AT^MONSC", Duration::from_secs(1)).unwrap();
    assert!(raw.contains("^MONSC: NR,78"));
    assert_eq!(*written.lock().unwrap(), b"AT^MONSC\r");
    assert!(!channel.poisoned());
    assert_eq!(channel.take_notices(), 1);
    assert_eq!(channel.take_notices(), 0);
}
#[test]
fn uncertain_timeout_quarantines_and_does_not_retry_or_accept_late_ok() {
    let (mut channel, written) = fake(&["^MONSC: partial\r\n"]);
    assert!(channel.query("AT^MONSC", Duration::from_secs(1)).is_err());
    assert!(channel.poisoned());
    let before = written.lock().unwrap().clone();
    assert_eq!(
        channel
            .query("AT^MONSC", Duration::from_secs(1))
            .unwrap_err()
            .code,
        "AT_CHANNEL_QUARANTINED"
    );
    assert_eq!(*written.lock().unwrap(), before);
}
#[test]
fn full_explicit_error_is_not_a_timeout_and_two_results_are_ambiguous() {
    let (mut channel, _) = fake(&["\r\n+CME ERROR: 4\r\n", "\r\nOK\r\n"]);
    assert_eq!(
        channel
            .query("AT^MONSC", Duration::from_secs(1))
            .unwrap_err()
            .code,
        "AT_UNSUPPORTED"
    );
    assert!(!channel.poisoned());
    assert!(channel.query("AT", Duration::from_secs(1)).is_ok());
    let (mut channel, _) = fake(&["\r\nOK\r\nOK\r\n"]);
    assert_eq!(
        channel
            .query("AT", Duration::from_secs(1))
            .unwrap_err()
            .code,
        "AT_FRAME_AMBIGUOUS"
    );
    assert!(channel.poisoned());
}
#[test]
fn sms_prompt_payload_and_reference_are_one_non_replayed_transaction() {
    let (mut channel, written) = fake(&["\r\n> ", "\r\n+CMGS: 17\r\nOK\r\n"]);
    assert!(
        channel
            .send_pdu("00010203", 3)
            .unwrap()
            .contains("+CMGS: 17")
    );
    assert_eq!(*written.lock().unwrap(), b"AT+CMGS=3\r00010203\x1a");
    let (mut channel, written) = fake(&["\r\n> "]);
    assert!(channel.send_pdu("00010203", 3).is_err());
    let sent = written.lock().unwrap().clone();
    assert!(channel.send_pdu("00010203", 3).is_err());
    assert_eq!(*written.lock().unwrap(), sent);
}
#[test]
fn invalid_payloads_and_command_injection_never_write() {
    for (pdu, n) in [("00010203", 2), ("00\x1a00", 2), ("0", 1), ("zz01", 1)] {
        let (mut channel, written) = fake(&[]);
        assert!(channel.send_pdu(pdu, n).is_err());
        assert!(written.lock().unwrap().is_empty());
    }
    let (mut channel, written) = fake(&[]);
    assert!(channel.query("AT;reboot", Duration::from_secs(1)).is_err());
    assert!(written.lock().unwrap().is_empty());
}
#[test]
fn ambiguous_or_invalid_send_reference_quarantines_without_replaying_paid_send() {
    for reply in [
        "+CMGS:1\r\n+CMGS:2\r\nOK\r\n",
        "+CMGS:256\r\nOK\r\n",
        "+CMGS:-1\r\nOK\r\n",
    ] {
        let (mut channel, written) = fake(&["> ", reply]);
        assert_eq!(
            channel.send_pdu("00010203", 3).err().unwrap().code,
            "SMS_SEND_UNKNOWN"
        );
        assert!(channel.poisoned());
        let first = written.lock().unwrap().clone();
        assert!(channel.send_pdu("00010203", 3).is_err());
        assert_eq!(*written.lock().unwrap(), first);
    }
}
#[test]
fn idle_cmti_is_fragmented_bounded_and_never_sends_at() {
    let (mut channel, written) = fake(&["+CMTI: \"M", "E\",7\r\n+CEREG: 1\r\n"]);
    channel.poll_notices(Duration::from_millis(50)).unwrap();
    assert_eq!(channel.take_notices(), 1);
    channel.poll_notices(Duration::from_millis(50)).unwrap();
    assert_eq!(channel.take_notices(), 0);
    assert!(written.lock().unwrap().is_empty());
    assert!(!channel.poisoned());
}
#[test]
fn a_fragment_after_final_cannot_become_the_next_query_response() {
    let (mut channel, written) = fake(&["^MONSC: NR,78\r\nOK\r\n+CMTI: \"ME\",", "8\r\n"]);
    channel.query("AT^MONSC", Duration::from_secs(1)).unwrap();
    assert_eq!(channel.take_notices(), 0);
    channel.poll_notices(Duration::from_millis(50)).unwrap();
    assert_eq!(channel.take_notices(), 1);
    assert_eq!(*written.lock().unwrap(), b"AT^MONSC\r");
    assert!(!channel.poisoned());
    let (mut channel, written) = fake(&["OK\r\n+CMTI: \"ME\",", "OK\r\n"]);
    channel.query("AT", Duration::from_secs(1)).unwrap();
    assert!(channel.query("AT+CGMM", Duration::from_secs(1)).is_err());
    assert_eq!(*written.lock().unwrap(), b"AT\r");
    assert!(channel.poisoned());
}
#[test]
fn late_transaction_results_and_overlong_idle_frames_are_quarantined() {
    for raw in [
        "OK\r\n",
        "+CMGS: 12\r\n",
        "> \r\n",
        "unbound reply\r\n",
        &"x".repeat(513),
    ] {
        let (mut channel, written) = fake(&[raw]);
        assert!(channel.poll_notices(Duration::from_millis(50)).is_err());
        assert!(channel.poisoned());
        assert_eq!(channel.take_notices(), 0);
        assert!(written.lock().unwrap().is_empty());
    }
}
#[test]
fn independent_send_accepts_only_its_own_exact_pdu_echo() {
    let (mut channel, _) = fake(&["> ", "00010203\u{1a}\r\n+CMGS: 7\r\nOK\r\n"]);
    assert!(channel.send_pdu("00010203", 3).is_ok());
    let (mut channel, _) = fake(&["> ", "00010204\u{1a}\r\n+CMGS: 7\r\nOK\r\n"]);
    assert_eq!(
        channel.send_pdu("00010203", 3).unwrap_err().code,
        "SMS_SEND_UNKNOWN"
    );
    assert!(channel.poisoned());
}
