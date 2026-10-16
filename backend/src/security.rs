//! 插件登录与一次性确认；不包含小程序商业授权逻辑。
use crate::{PanelError, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::time::Duration;
use subtle::ConstantTimeEq;

pub trait Ubus {
    fn call(&mut self, object: &str, method: &str, params: Value) -> Result<Value>;
}
type Key = [u8; 32];
fn digest(v: &str) -> Key {
    Sha256::digest(v.as_bytes()).into()
}
fn equal(a: &Key, b: &Key) -> bool {
    bool::from(a.ct_eq(b))
}
fn nonce(bytes: usize) -> Result<String> {
    // 只用系统安全随机数；不得用时间、序号或模组身份生成会话凭据。
    let mut raw = vec![0; bytes];
    getrandom::fill(&mut raw).map_err(|_| PanelError::new("RANDOM_UNAVAILABLE", 503))?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}
struct Session {
    peer: Ipv4Addr,
    expires: Duration,
}
struct Challenge {
    owner: Key,
    modem: String,
    command: Key,
    expires: Duration,
}
#[derive(Default)]
pub struct Security {
    sessions: HashMap<Key, Session>,
    failures: HashMap<Ipv4Addr, VecDeque<Duration>>,
    challenges: HashMap<String, Challenge>,
}
impl Security {
    fn prune(&mut self, now: Duration) {
        self.sessions.retain(|_, s| s.expires > now);
        self.challenges.retain(|_, s| s.expires > now);
        self.failures.retain(|_, attempts| {
            while attempts
                .front()
                .is_some_and(|at| now.saturating_sub(*at) >= Duration::from_secs(300))
            {
                attempts.pop_front();
            }
            !attempts.is_empty()
        });
    }
    pub fn login(
        &mut self,
        ubus: &mut impl Ubus,
        username: &str,
        password: &str,
        peer: Ipv4Addr,
        now: Duration,
    ) -> Result<Value> {
        // 无效用户名也计入限流；系统会话只做权限核验，随后销毁。
        self.prune(now);
        if self.failures.get(&peer).is_some_and(|v| v.len() >= 5)
            || self.failures.values().map(|v| v.len()).sum::<usize>() >= 40
        {
            return Err(PanelError::new("LOGIN_RATE_LIMIT", 429));
        }
        self.failures.entry(peer).or_default().push_back(now);
        if username != "root" || password.is_empty() || password.chars().count() > 256 {
            return Err(PanelError::new("LOGIN_FAILED", 401));
        }
        let response = ubus
            .call(
                "session",
                "login",
                json!({"username":username,"password":password,"timeout":60}),
            )
            .map_err(|_| PanelError::new("LOGIN_FAILED", 401))?;
        let sid = response["ubus_rpc_session"]
            .as_str()
            .filter(|s| s.len() == 32)
            .ok_or_else(|| PanelError::new("LOGIN_FAILED", 401))?;
        let access = ubus.call(
            "session",
            "access",
            json!({"ubus_rpc_session":sid,"scope":"ubus","object":"system","function":"board"}),
        );
        ubus.call("session", "destroy", json!({"ubus_rpc_session":sid}))
            .map_err(|_| PanelError::new("LOGIN_FAILED", 401))?;
        if access.map(|v| v["access"] != true).unwrap_or(true) {
            return Err(PanelError::new("LOGIN_FAILED", 403));
        }
        if self.sessions.len() >= 16 {
            return Err(PanelError::new("SESSION_LIMIT", 429));
        }
        let token = nonce(32)?;
        self.sessions.insert(
            digest(&token),
            Session {
                peer,
                expires: now + Duration::from_secs(1800),
            },
        );
        self.failures.remove(&peer);
        Ok(json!({"token":token,"expiresIn":1800}))
    }
    pub fn authenticate(&mut self, token: &str, peer: Ipv4Addr, now: Duration) -> Result<String> {
        self.prune(now);
        if !(30..=80).contains(&token.len()) {
            return Err(PanelError::new("SESSION_EXPIRED", 401));
        }
        let key = digest(token);
        if !self.sessions.get(&key).is_some_and(|s| s.peer == peer) {
            return Err(PanelError::new("SESSION_EXPIRED", 401));
        }
        Ok(key.iter().map(|b| format!("{b:02x}")).collect())
    }
    pub fn logout(&mut self, token: &str) {
        let key = digest(token);
        self.sessions.remove(&key);
        let owner = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let owner = digest(&owner);
        self.challenges.retain(|_, c| !equal(&c.owner, &owner));
    }
    pub fn prepare(
        &mut self,
        owner: &str,
        modem: &str,
        command: &str,
        now: Duration,
    ) -> Result<Value> {
        self.prune(now);
        if owner.is_empty() || owner.len() > 80 || modem.len() > 128 || command.len() > 512 {
            return Err(PanelError::new("AT_INVALID", 400));
        }
        if self.challenges.len() >= 32 {
            return Err(PanelError::new("AT_RATE_LIMIT", 429));
        }
        let nonce = nonce(24)?;
        self.challenges.insert(
            nonce.clone(),
            Challenge {
                owner: digest(owner),
                modem: modem.to_owned(),
                command: digest(command),
                expires: now + Duration::from_secs(30),
            },
        );
        Ok(
            json!({"confirmation":nonce,"expiresIn":30,"warning":"该指令可能改变模组配置或导致断网。仅在理解指令含义时继续。"}),
        )
    }
    pub fn consume(
        &mut self,
        nonce: &str,
        owner: &str,
        modem: &str,
        command: &str,
        now: Duration,
    ) -> Result<()> {
        // 先消费一次性确认，后校验绑定；失败也不能重用以重放写操作。
        self.prune(now);
        let value = self
            .challenges
            .remove(nonce)
            .ok_or_else(|| PanelError::new("AT_CONFIRMATION_REQUIRED", 409))?;
        if !equal(&value.owner, &digest(owner))
            || value.modem != modem
            || !equal(&value.command, &digest(command))
        {
            return Err(PanelError::new("AT_CONFIRMATION_REQUIRED", 409));
        }
        Ok(())
    }
}

pub fn read_only(command: &str) -> bool {
    [
        "AT",
        "ATI",
        "AT+CGMI",
        "AT+CGMM",
        "AT+CGMR",
        "AT+CGSN",
        "AT+CSQ",
        "AT+CESQ",
        "AT+COPS?",
        "AT+CPIN?",
        "AT+CMGF?",
        "AT+CPMS?",
        "AT+CSCS?",
        "AT+CSDH?",
        "AT+CNMI?",
        "AT+CSMS?",
        "AT+CEREG?",
        "AT+CREG?",
        "AT+CGATT?",
        "AT^DSAMBR=1",
        "AT^DSAMBR=8",
        "AT+CGEQOSRDP",
        "AT+CGCONTRDP",
        "AT^IMSSWITCH?",
        "AT+QNWCFG=\"NR5G_AMBR\"",
        "AT^NRFREQLOCK?",
        "AT^LTEFREQLOCK?",
        "AT+QNWLOCK=\"COMMON/5G\"",
        "AT+QNWLOCK=\"COMMON/LTE\"",
        "AT^MONSC",
        "AT^HFREQINFO?",
        "AT^MONNC",
        "AT^CHIPTEMP?",
        "AT+QTEMP",
        "AT+MTSM=1",
        "AT+MTSM=1,6",
        "AT+GTLADC",
        "AT+GTSENRDTEMP=1",
        "AT+TEMP",
        "AT+TEMP?",
        "AT^TEMP?",
        "AT!PCTEMP?",
        "AT!BAND?",
        "AT!BAND=?",
        "AT+CPMUTEMP",
        "AT#TEMPSENS=2",
        "AT+QENG=\"SERVINGCELL\"",
        "AT+QCAINFO",
        "AT+QENG=\"NEIGHBOURCELL\"",
        "AT+GTCCINFO?",
        "AT+GTCAINFO?",
        "AT^CELLINFO=1",
        "AT^CSERSSI?",
        "AT+DEBUG?",
        "AT^DEBUG?",
        "AT+NETDMSGEX",
        "AT!GSTATUS?",
        "AT!LTEINFO?",
        "AT!NRINFO?",
        "AT#CAINFOEXT?",
        "AT+CPSI?",
        "AT+ZCELLINFO?",
        "AT+CCED=0,2",
        "AT+CCED=0,1",
    ]
    .contains(&command.to_ascii_uppercase().as_str())
}
pub fn validate_at(command: &str) -> Result<()> {
    if command.len() < 2
        || command.len() > 192
        || command
            .bytes()
            .any(|b| !(0x20..=0x7e).contains(&b) || b == b';')
        || !command[..2].eq_ignore_ascii_case("AT")
    {
        return Err(PanelError::new("AT_INVALID", 400));
    }
    Ok(())
}
