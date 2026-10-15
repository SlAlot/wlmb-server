//! 脱敏诊断事件的有界环形记录；不保存AT正文或账户凭据。
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Default)]
struct State {
    last: HashMap<(String, String), Instant>,
    recent: VecDeque<Value>,
}
#[derive(Default)]
pub struct Events {
    state: Mutex<State>,
}
pub fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |v| v.as_millis().min(u64::MAX as u128) as u64)
}
impl Events {
    pub fn record(&self, source: &str, code: &str) {
        if ![
            "http",
            "local",
            "sampler",
            "discovery",
            "host",
            "startup",
            "sms-forward",
            "settings",
            "host-settings",
        ]
        .contains(&source)
        {
            return;
        }
        let code = if !code.is_empty()
            && code.len() <= 48
            && code.as_bytes()[0].is_ascii_uppercase()
            && code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            code
        } else {
            "UNCLASSIFIED_ERROR"
        };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state
            .last
            .retain(|_, stamp| stamp.elapsed() < Duration::from_secs(300));
        let key = (source.to_owned(), code.to_owned());
        if state
            .last
            .get(&key)
            .is_some_and(|stamp| stamp.elapsed() < Duration::from_secs(30))
        {
            return;
        }
        if state.last.len() >= 64 {
            if let Some(old) = state
                .last
                .iter()
                .min_by_key(|(_, stamp)| **stamp)
                .map(|(key, _)| key.clone())
            {
                state.last.remove(&old);
            }
        }
        state.last.insert(key, Instant::now());
        if state.recent.len() >= 32 {
            state.recent.pop_front();
        }
        state
            .recent
            .push_back(json!({"source":source,"code":code,"atMs":wall_ms()}));
        eprintln!("wlmb-server: {source}: {code}");
    }
    pub fn recent(&self) -> Value {
        self.state.lock().map_or(json!([]), |s| json!(s.recent))
    }
}
