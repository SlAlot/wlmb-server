//! 制式编码参考 QModem 公开厂商脚本；本地只生成已验证的选网命令。
use super::*;

const MODES: [&str; 5] = ["auto", "5g", "4g", "3g", "2g"];
const Q_NR: &[(&str, &str)] = &[
    ("AUTO", "auto"),
    ("NR5G", "5g"),
    ("LTE", "4g"),
    ("WCDMA", "3g"),
    ("LTE:NR5G", "4g+5g"),
    ("WCDMA:LTE", "3g+4g"),
    ("WCDMA:NR5G", "3g+5g"),
];
const Q_LTE: &[(&str, &str)] = &[("0", "auto"), ("3", "4g"), ("2", "3g"), ("1", "2g")];
const CNMP: &[(&str, &str)] = &[
    ("2", "auto"),
    ("71", "5g"),
    ("38", "4g"),
    ("14", "3g"),
    ("13", "2g"),
    ("54", "3g+4g"),
    ("55", "3g+5g"),
    ("109", "4g+5g"),
];
const SIERRA: &[(&str, &str)] = &[
    ("00", "auto"),
    ("20", "5g"),
    ("06", "4g"),
    ("01", "3g"),
    ("11", "3g+4g"),
    ("21", "4g+5g"),
    ("22", "3g+5g"),
];
const WS46: &[(&str, &str)] = &[
    ("38", "auto"),
    ("36", "5g"),
    ("28", "4g"),
    ("22", "3g"),
    ("31", "3g+4g"),
    ("37", "4g+5g"),
    ("40", "3g+5g"),
];
const HUAWEI: &[(&str, &str)] = &[
    ("00", "auto"),
    ("08", "5g"),
    ("03", "4g"),
    ("02", "3g"),
    ("01", "2g"),
];
const MEIG: &[(&str, &str)] = &[
    ("00", "auto"),
    ("04", "5g"),
    ("03", "4g"),
    ("02", "3g"),
    ("01", "2g"),
];
const GTACT_NR: &[(&str, &str)] = &[
    ("20", "auto"),
    ("14", "5g"),
    ("2", "4g"),
    ("1", "3g"),
    ("4", "3g+4g"),
    ("16", "3g+5g"),
    ("17", "4g+5g"),
];
const GTACT_LTE: &[(&str, &str)] = &[("10", "auto"), ("2", "4g"), ("1", "3g"), ("4", "3g+4g")];
const CTEC: &[(&str, &str)] = &[("0", "auto"), ("4", "4g"), ("2", "2g")];
const SLMODE: &[(&str, &str)] = &[
    ("0", "auto"),
    ("7", "3g+4g+5g"),
    ("4", "5g"),
    ("2", "4g"),
    ("1", "3g"),
    ("3", "3g+4g"),
    ("5", "3g+5g"),
    ("6", "4g+5g"),
];
const ZSNT: &[(&str, &str)] = &[
    ("0", "auto"),
    ("7", "5g"),
    ("6", "4g"),
    ("2", "3g"),
    ("8", "4g+5g"),
];
const MYSYS: &[(&str, &str)] = &[
    ("1", "auto"),
    ("9", "5g"),
    ("5", "5g"),
    ("4", "4g"),
    ("3", "3g"),
    ("7", "3g+4g"),
    ("11", "3g+5g"),
    ("12", "4g+5g"),
    ("14", "3g+4g+5g"),
];

struct Protocol {
    query: &'static str,
    test: &'static str,
    prefix: &'static str,
    set: &'static str,
    codes: &'static [(&'static str, &'static str)],
    base: &'static [&'static str],
    layout: &'static str,
}
fn protocol(model: &str) -> Option<Protocol> {
    let profile = crate::catalog::identify(model);
    let key = profile["model"].as_str().unwrap_or("");
    let platform = crate::catalog::platform(key);
    let (query, test, prefix, set, codes, base, layout) = match profile["family"].as_str()? {
        "quectel" if platform == "lte" => (
            "AT+QCFG=\"nwscanmode\"",
            "AT+QCFG=?",
            "+QCFG:",
            "AT+QCFG=\"nwscanmode\",",
            Q_LTE,
            &["auto", "4g"][..],
            "key",
        ),
        "quectel" => (
            "AT+QNWPREFCFG=\"mode_pref\"",
            "AT+QNWPREFCFG=?",
            "+QNWPREFCFG:",
            "AT+QNWPREFCFG=\"mode_pref\",",
            Q_NR,
            if platform == "lte12" {
                &["auto", "4g"][..]
            } else {
                &["auto", "4g", "5g"][..]
            },
            "key",
        ),
        "simcom" => (
            "AT+CNMP?",
            "AT+CNMP=?",
            "+CNMP:",
            "AT+CNMP=",
            CNMP,
            &[][..],
            "plain",
        ),
        "sierra" => (
            "AT!SELRAT?",
            "AT!SELRAT=?",
            "!SELRAT:",
            "AT!SELRAT=",
            SIERRA,
            &[][..],
            "sierra",
        ),
        "telit" => (
            "AT+WS46?",
            "AT+WS46=?",
            "+WS46:",
            "AT+WS46=",
            WS46,
            &[][..],
            "plain",
        ),
        "tdtech" => (
            "AT^SYSCFGEX?",
            "AT^SYSCFGEX=?",
            "^SYSCFGEX:",
            "AT^SYSCFGEX=",
            HUAWEI,
            &[][..],
            "syscfg",
        ),
        "meig" => (
            "AT^SYSCFGEX?",
            "AT^SYSCFGEX=?",
            "^SYSCFGEX:",
            "AT^SYSCFGEX=",
            MEIG,
            &[][..],
            "syscfg",
        ),
        "fibocom" => (
            "AT+GTACT?",
            "AT+GTACT=?",
            "+GTACT:",
            "AT+GTACT=",
            if platform == "lte" {
                GTACT_LTE
            } else {
                GTACT_NR
            },
            &[][..],
            "gtact",
        ),
        "openluat" => (
            "AT+CTEC?",
            "",
            "+CTEC:",
            "AT+CTEC=",
            CTEC,
            &["auto", "4g", "2g"][..],
            "ctec",
        ),
        "thales" | "foxconn" => {
            let (query, test, prefix, set) = if ["t99w368", "t99w640"].contains(&key) {
                ("AT+SLMODE?", "AT+SLMODE=?", "+SLMODE:", "AT+SLMODE=1,")
            } else {
                ("AT^SLMODE?", "AT^SLMODE=?", "^SLMODE:", "AT^SLMODE=1,")
            };
            (query, test, prefix, set, SLMODE, &[][..], "slmode")
        }
        "gosuncn" => (
            "AT+ZSNT?",
            "AT+ZSNT=?",
            "+ZSNT:",
            "AT+ZSNT=",
            ZSNT,
            &[][..],
            "zsnt",
        ),
        "neoway" => (
            "AT$MYSYSINFO",
            "AT$MYSYSINFO=?",
            "$MYSYSINFO:",
            "AT$MYSYSINFO=",
            MYSYS,
            &[][..],
            "mysys",
        ),
        _ => return None,
    };
    Some(Protocol {
        query,
        test,
        prefix,
        set,
        codes,
        base,
        layout,
    })
}
fn at_ready(row: &Value) -> bool {
    row["controlReadOnly"] != true
        && row["channelState"] != "sms-input-unknown"
        && (row["shared"] == true
            || row["transport"] == "dedicated"
                && row["channelReady"] == true
                && row["identityVerified"] == true)
}
#[cfg(unix)]
pub(crate) fn qmodem_model(model: &str) -> bool {
    crate::catalog::identify(model)["family"] == "quectel"
        && crate::catalog::platform(model) == "qualcomm"
}
fn unsupported() -> PanelError {
    error("NETWORK_MODE_UNSUPPORTED", 422)
}
fn unknown() -> PanelError {
    error("NETWORK_MODE_UNKNOWN", 409)
}
fn unverified() -> PanelError {
    error("NETWORK_MODE_UNVERIFIED", 422)
}
fn busy() -> PanelError {
    error("AT_BUSY", 409)
}
fn ok(raw: &str) -> bool {
    raw.len() <= crate::MAX_AT_RESPONSE
        && !raw.contains('\0')
        && raw
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| line.trim() == "OK")
        && !raw.lines().map(str::trim).any(|line| {
            line == "ERROR" || line.starts_with("+CME ERROR") || line.starts_with("+CMS ERROR")
        })
}
fn clean(field: &str) -> &str {
    let field = field.trim();
    field
        .strip_prefix('"')
        .and_then(|f| f.strip_suffix('"'))
        .unwrap_or(field)
}
fn fields<'a>(p: &Protocol, raw: &'a str) -> Option<Vec<&'a str>> {
    if !ok(raw) {
        return None;
    }
    let mut records = raw
        .lines()
        .filter_map(|line| line.trim().strip_prefix(p.prefix))
        .filter_map(|line| {
            if p.layout == "key" {
                let (key, rest) = line.split_once(',')?;
                if !p.query.ends_with(&format!("\"{}\"", clean(key))) {
                    return None;
                }
                Some(rest)
            } else {
                Some(line)
            }
        });
    let record = records.next()?;
    if records.next().is_some() {
        return None;
    }
    Some(record.split(',').map(clean).collect())
}
fn label(mode: &str) -> String {
    if mode == "auto" {
        "自动选网".into()
    } else if MODES.contains(&mode) {
        format!("仅 {}", mode.to_ascii_uppercase())
    } else {
        mode.to_ascii_uppercase().replace('+', " + ")
    }
}
fn decode(p: &Protocol, code: &str) -> Option<String> {
    if let Some((_, mode)) = p.codes.iter().find(|(k, _)| *k == code) {
        return Some((*mode).into());
    }
    if p.codes == Q_NR {
        let mut rats = vec![];
        for token in code.split(':') {
            let (_, rat) = Q_NR
                .iter()
                .find(|(k, m)| *k == token && MODES.contains(m) && *m != "auto")?;
            if rats.contains(rat) {
                return None;
            }
            rats.push(*rat);
        }
        return (!rats.is_empty()).then(|| rats.join("+"));
    }
    if p.layout != "syscfg" || code.len() > 8 || !code.is_ascii() || code.len() % 2 != 0 {
        return None;
    }
    let mut rats = vec![];
    for i in (0..code.len()).step_by(2) {
        let (_, rat) = p
            .codes
            .iter()
            .find(|(k, m)| *k == &code[i..i + 2] && *m != "auto")?;
        if rats.contains(rat) {
            return None;
        }
        rats.push(*rat);
    }
    (!rats.is_empty()).then(|| rats.join("+"))
}
fn tail(p: &Protocol, f: &[&str]) -> Option<String> {
    match p.layout {
        "syscfg" => {
            let count = if p.codes == HUAWEI { 5 } else { 9 };
            if f.len() < count
                || f.len() > count + 2
                || f[count..].iter().any(|x| !x.is_empty())
                || !f[1..count].iter().all(|v| {
                    *v == "all"
                        || !v.is_empty()
                            && v.len() <= 32
                            && v.bytes().all(|b| b.is_ascii_hexdigit())
                })
                || ![2, 3]
                    .iter()
                    .all(|i| f[*i].len() == 1 && f[*i].bytes().all(|b| b.is_ascii_digit()))
            {
                return None;
            }
            let suffix = if count == 5 { ",," } else { "" };
            Some(format!(",{}{suffix}", f[1..count].join(",")))
        }
        "gtact" => {
            // 带频段参数时制式切换可能改变频段配置；未证明可保留时仅显示。
            if f.len() > 3
                || f[1..].iter().any(|v| {
                    !v.is_empty() && (v.len() > 2 || !v.bytes().all(|b| b.is_ascii_digit()))
                })
            {
                return None;
            }
            Some(if f.len() > 1 {
                format!(",{}", f[1..].join(","))
            } else {
                String::new()
            })
        }
        "ctec" => (f.len() == 2).then(String::new),
        "slmode" => (f.len() == 2 && f[0] == "1").then(String::new),
        "zsnt" => (f.len() == 3
            && f[1..]
                .iter()
                .all(|n| n.len() == 1 && n.bytes().all(|b| b.is_ascii_digit())))
        .then(|| format!(",{},{}", f[1], f[2])),
        "plain" | "sierra" => (f.len() == 1).then(String::new),
        _ => (f.len() == 1).then(String::new),
    }
}
fn advertised(p: &Protocol, raw: &str) -> Option<Vec<String>> {
    if !ok(raw) {
        return None;
    }
    let key = p
        .query
        .split_once("=\"")
        .map(|(_, k)| k.trim_end_matches('"'));
    let mut value = raw.lines().find_map(|line| {
        let line = line.trim().strip_prefix(p.prefix)?.trim();
        if let Some(key) = key {
            let (first, values) = line.split_once(',')?;
            if clean(first.trim_start_matches('(')) != key {
                return None;
            }
            Some(values.trim())
        } else {
            Some(line)
        }
    })?;
    if p.layout == "sierra" {
        let mut codes = Vec::new();
        for line in raw.lines() {
            let entry = line.trim().trim_start_matches("!SELRAT:").trim();
            if let Some(code) = entry
                .split_whitespace()
                .next()
                .filter(|code| p.codes.iter().any(|(k, _)| k == code))
            {
                codes.push(code.to_owned());
            }
        }
        if !codes.is_empty() {
            return Some(codes);
        }
    }
    if p.layout == "slmode" && value.starts_with('(') {
        value = value.split_once(')')?.1.trim().strip_prefix(',')?.trim();
    }
    if let Some(list) = value.strip_prefix('(') {
        value = list.split_once(')')?.0;
    } else if let Some((list, _)) = value.split_once(')') {
        value = list;
    }
    let tokens = value
        .split(',')
        .map(clean)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    let codes = p
        .codes
        .iter()
        .filter(|(code, _)| {
            tokens.iter().any(|token| {
                if token == code {
                    return true;
                }
                if p.layout == "syscfg" {
                    return false;
                }
                let Some((lo, hi)) = token.split_once('-') else {
                    return false;
                };
                matches!(
                    (lo.parse::<u8>(), hi.parse::<u8>(), code.parse::<u8>()),
                    (Ok(lo), Ok(hi), Ok(n)) if lo <= n && n <= hi
                )
            })
        })
        .map(|(code, _)| (*code).to_owned())
        .collect::<Vec<_>>();
    (!codes.is_empty()).then_some(codes)
}
struct Reading {
    mode: String,
    wire: String,
    options: Vec<(&'static str, String)>,
    native: bool,
}
fn direct(p: &Protocol, raw: &str, test: Option<&str>) -> Result<Reading> {
    let f = fields(p, raw).ok_or_else(unverified)?;
    let first = f.first().ok_or_else(unverified)?;
    let code = match p.layout {
        "sierra" => first.split_whitespace().next().unwrap_or(""),
        "slmode" if *first == "1" => f.get(1).copied().ok_or_else(unverified)?,
        "slmode" => return Err(unverified()),
        "ctec" => f.get(1).copied().ok_or_else(unverified)?,
        _ => *first,
    };
    let mode = decode(p, code).ok_or_else(unverified)?;
    let allowed = test.and_then(|raw| advertised(p, raw));
    let tail = tail(p, &f);
    let mut options = vec![];
    for requested in MODES {
        if requested == "5g" && p.codes == Q_NR && !p.base.contains(&"5g") {
            continue;
        }
        let Some((k, _)) = p.codes.iter().find(|(k, m)| {
            *m == requested
                && allowed.as_ref().map_or(p.base.contains(m), |a| {
                    a.iter().any(|a| a == k) || mode == requested
                })
        }) else {
            continue;
        };
        if let Some(tail) = &tail {
            let args = match p.layout {
                "ctec" => format!("{k},{k}"),
                "syscfg" => format!("\"{k}\"{tail}"),
                _ => format!("{k}{tail}"),
            };
            let command = format!("{}{args}", p.set);
            if crate::security::validate_at(&command).is_ok() {
                options.push((requested, command));
            }
        }
    }
    if !options.iter().any(|(option, _)| *option == "auto") {
        // 未确认能切回自动时，不开放把设备限制到单一制式。
        options.clear();
    }
    Ok(Reading {
        mode,
        wire: tail.unwrap_or_else(|| f[1..].join(",")),
        options,
        native: false,
    })
}

pub(crate) const QMODEM_MODES: [&str; 4] = ["auto", "5g", "4g", "3g"];
#[cfg(unix)]
pub(crate) fn qmodem_params(mode: &str) -> Option<&'static str> {
    match mode {
        "auto" => Some(r#"["3G","4G","5G"]"#),
        "5g" => Some(r#"["5G"]"#),
        "4g" => Some(r#"["4G"]"#),
        "3g" => Some(r#"["3G"]"#),
        _ => None,
    }
}
fn native(value: &Value) -> Result<Reading> {
    let entries = value["network_prefer"].as_object().ok_or_else(unverified)?;
    let mut rats = vec![];
    if entries.len() != 3 {
        return Err(unverified());
    }
    for rat in ["3G", "4G", "5G"] {
        match entries.get(rat).and_then(Value::as_str) {
            Some("1") => rats.push(rat.to_ascii_lowercase()),
            Some("0") => {}
            _ => return Err(unverified()),
        }
    }
    if rats.is_empty() {
        return Err(unverified());
    }
    Ok(Reading {
        mode: if rats.len() == 3 {
            "auto".into()
        } else {
            rats.join("+")
        },
        wire: String::new(),
        options: QMODEM_MODES.iter().map(|m| (*m, String::new())).collect(),
        native: true,
    })
}
impl RadioControls {
    pub(crate) fn network_mode_profile(model: &str) -> bool {
        protocol(model).is_some()
    }
    pub fn network_mode_supported(row: &Value) -> bool {
        (at_ready(row) && Self::network_mode_profile(row["model"].as_str().unwrap_or("")))
            || row["qmodemNetworkMode"] == true
                && row["controlReadOnly"] != true
                && row["channelState"] != "sms-input-unknown"
    }
    fn mode_row(&self, key: &str) -> Result<Value> {
        let rows = self.source.modems()?;
        let mut matches = rows.into_iter().filter(|r| r["id"] == key);
        let row = matches
            .next()
            .filter(Self::network_mode_supported)
            .ok_or_else(unsupported)?;
        if matches.next().is_some() {
            return Err(unsupported());
        }
        Ok(row)
    }
    fn mode_read_locked(&self, row: &Value, probe: bool) -> Result<Reading> {
        if row["qmodemNetworkMode"] == true {
            return native(&self.source.network_preference(row, None)?);
        }
        let p = protocol(row["model"].as_str().unwrap_or("")).ok_or_else(unsupported)?;
        let raw = self.source.query_bound(row, p.query)?;
        let test = if probe && !p.test.is_empty() {
            match self.source.query_bound(row, p.test) {
                Ok(value) => Some(value),
                Err(e) if e.code == "AT_UNSUPPORTED" => None,
                Err(e) => return Err(e),
            }
        } else {
            None
        };
        direct(&p, &raw, test.as_deref())
    }
    fn mode_revision(row: &Value, state: &Reading) -> String {
        let bytes = json!([
            row["id"],
            row["port"],
            row["model"],
            row["instance"],
            state.mode,
            state.wire,
            state.options,
            state.native
        ])
        .to_string();
        format!("{:x}", Sha256::digest(bytes.as_bytes()))
    }
    fn mode_public(row: &Value, state: &Reading) -> Value {
        json!({"modem":row["id"],"mode":if MODES.contains(&state.mode.as_str()){json!(state.mode)}else{Value::Null},
            "label":label(&state.mode),"writable":!state.options.is_empty(),
            "options":state.options.iter().map(|(mode,_)|json!({"mode":mode,"label":label(mode)})).collect::<Vec<_>>(),
            "revision":Self::mode_revision(row,state)})
    }
    pub fn network_mode_read(&self, key: &str) -> Result<Value> {
        let _held = self.gate.try_lock().map_err(|_| busy())?;
        let row = self.mode_row(key)?;
        let state = self.mode_read_locked(&row, true)?;
        Ok(Self::mode_public(&row, &state))
    }
    pub fn network_mode_prepare(
        &self,
        key: &str,
        mode: &str,
        revision: &str,
        owner: &str,
    ) -> Result<Value> {
        self.writable()?;
        let _held = self.gate.try_lock().map_err(|_| busy())?;
        let row = self.mode_row(key)?;
        let state = self.mode_read_locked(&row, true)?;
        if revision != Self::mode_revision(&row, &state) {
            return Err(error("NETWORK_MODE_CHANGED", 409));
        }
        if !state.options.iter().any(|(option, _)| *option == mode) {
            return Err(unsupported());
        }
        let mut prepared = self.security.lock().map_err(|_| busy())?.prepare(
            owner,
            key,
            &format!("NETWORK_MODE:{revision}:{mode}"),
            self.started.elapsed(),
        )?;
        prepared["revision"] = json!(revision);
        Ok(prepared)
    }
    pub fn network_mode_execute(
        &self,
        key: &str,
        mode: &str,
        revision: &str,
        confirmation: &str,
        owner: &str,
    ) -> Result<Value> {
        self.writable()?;
        if revision.len() != 64 || !MODES.contains(&mode) {
            return Err(unsupported());
        }
        let _held = self.gate.try_lock().map_err(|_| busy())?;
        self.security.lock().map_err(|_| busy())?.consume(
            confirmation,
            owner,
            key,
            &format!("NETWORK_MODE:{revision}:{mode}"),
            self.started.elapsed(),
        )?;
        let row = self.mode_row(key)?;
        let state = self.mode_read_locked(&row, true)?;
        if revision != Self::mode_revision(&row, &state) {
            return Err(error("NETWORK_MODE_CHANGED", 409));
        }
        let (_, command) = state
            .options
            .iter()
            .find(|(m, _)| *m == mode)
            .ok_or_else(unsupported)?;
        if state.mode != mode {
            self.writable()?;
            self.cache.lock().map_err(|_| busy())?.remove(key);
            if state.native {
                self.source
                    .network_preference(&row, Some(mode))
                    .map_err(|_| unknown())?;
            } else {
                let raw = self
                    .source
                    .query_bound(&row, command)
                    .map_err(|_| unknown())?;
                if !controls::write_accepted(&raw, command) {
                    return Err(unknown());
                }
            }
        }
        let mut after = self.mode_read_locked(&row, false).map_err(|_| unknown())?;
        if after.mode != mode || after.wire != state.wire {
            return Err(unknown());
        }
        after.options = state.options;
        let mut result = Self::mode_public(&row, &after);
        result["verified"] = json!(true);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_and_firmware_gate_options_and_distinguish_configured_combinations() -> Result<()> {
        let p = protocol("RM520F-GL").ok_or_else(unsupported)?;
        let test = "+QNWPREFCFG: \"mode_pref\",(AUTO,WCDMA,LTE,NR5G)\r\nOK\r\n";
        let reading = direct(&p, "+QNWPREFCFG: \"mode_pref\",LTE:NR5G\nOK", Some(test))?;
        assert_eq!(reading.mode, "4g+5g");
        assert!(
            reading
                .options
                .contains(&("3g", "AT+QNWPREFCFG=\"mode_pref\",WCDMA".into()))
        );
        let p = protocol("EM160R-GL").ok_or_else(unsupported)?;
        assert!(
            !direct(&p, "+QNWPREFCFG: \"mode_pref\",AUTO\nOK", Some(test))?
                .options
                .iter()
                .any(|(m, _)| *m == "5g")
        );
        let p = protocol("EC25").ok_or_else(unsupported)?;
        let reading = direct(
            &p,
            "+QCFG: \"nwscanmode\",0\nOK",
            Some("+QCFG: (\"nwscanmode\",(0-3),(0,1))\nOK"),
        )?;
        assert!(reading.options.iter().any(|(m, _)| *m == "3g"));
        assert!(!reading.options.iter().any(|(m, _)| *m == "5g"));
        let p = protocol("A7605C").ok_or_else(unsupported)?;
        let reading = direct(&p, "+CNMP: 38\nOK", Some("+CNMP: (2,13,38)\nOK"))?;
        assert_eq!(
            reading.options.iter().map(|(m, _)| *m).collect::<Vec<_>>(),
            vec!["auto", "4g", "2g"]
        );
        assert!(
            direct(&p, "+CNMP: 38\nOK", Some("+CNMP: (14,38)\nOK"))?
                .options
                .is_empty()
        );
        assert!(direct(&p, "+CNMP: 38\n+CME ERROR: 3\nOK", None).is_err());
        assert!(protocol("unknown").is_none());
        Ok(())
    }

    #[test]
    fn shared_vendor_registers_do_not_reset_other_fields() -> Result<()> {
        let p = protocol("MT5700M-CN").ok_or_else(unsupported)?;
        let test = "^SYSCFGEX: (\"00\",\"03\",\"08\",\"0803\"),(40000000),(0,1),(2),(40000000)\nOK";
        let current = "^SYSCFGEX: \"0803\",00000A12,1,2,00000080\nOK";
        let reading = direct(&p, current, Some(test))?;
        assert_eq!(reading.mode, "5g+4g");
        assert!(
            reading
                .options
                .contains(&("5g", "AT^SYSCFGEX=\"08\",00000A12,1,2,00000080,,".into()))
        );
        assert!(!reading.options.iter().any(|(m, _)| *m == "3g"));
        assert!(
            direct(&p, "^SYSCFGEX: \"00\",broken,1,2,40000000\nOK", Some(test))?
                .options
                .is_empty()
        );
        let p = protocol("FM350-GL").ok_or_else(unsupported)?;
        assert!(
            direct(
                &p,
                "+GTACT: 17,6,6,103,5078\nOK",
                Some("+GTACT: (1,2,14,17,20),(6),(6)\nOK")
            )?
            .options
            .is_empty()
        );
        let p = protocol("T99W368").ok_or_else(unsupported)?;
        let reading = direct(&p, "+SLMODE: 1,7\nOK", Some("+SLMODE: (1),(0-7)\nOK"))?;
        assert_eq!(reading.mode, "3g+4g+5g");
        assert!(reading.options.contains(&("5g", "AT+SLMODE=1,4".into())));
        Ok(())
    }

    struct Mock {
        row: Mutex<Value>,
        mode: Mutex<String>,
        writes: Mutex<Vec<String>>,
        update: AtomicBool,
    }
    impl Source for Mock {
        fn modems(&self) -> Result<Vec<Value>> {
            Ok(vec![self.row.lock().map_err(|_| busy())?.clone()])
        }
        fn query(&self, _id: &str, cmd: &str, _port: &str, _model: &str) -> Result<String> {
            if cmd.ends_with("=?") {
                return Ok("+QNWPREFCFG: \"mode_pref\",(AUTO,WCDMA,LTE,NR5G)\nOK\n".into());
            }
            if cmd == "AT+QNWPREFCFG=\"mode_pref\"" {
                return Ok(format!(
                    "+QNWPREFCFG: \"mode_pref\",{}\nOK\n",
                    self.mode.lock().map_err(|_| busy())?
                ));
            }
            let (_, value) = cmd.split_once(',').ok_or_else(unverified)?;
            self.writes.lock().map_err(|_| busy())?.push(cmd.into());
            if self.update.load(Ordering::Acquire) {
                *self.mode.lock().map_err(|_| busy())? = value.into();
            }
            Ok("OK\n".into())
        }
        fn network_preference(&self, _row: &Value, mode: Option<&str>) -> Result<Value> {
            let mut current = self.mode.lock().map_err(|_| busy())?;
            if let Some(mode) = mode {
                self.writes.lock().map_err(|_| busy())?.push(mode.into());
                if self.update.load(Ordering::Acquire) {
                    *current = match mode {
                        "auto" => "AUTO",
                        "5g" => "NR5G",
                        "4g" => "LTE",
                        "3g" => "WCDMA",
                        _ => return Err(unsupported()),
                    }
                    .into();
                }
                return Ok(json!({}));
            }
            let flag = |rat: &str| {
                if *current == "AUTO" || *current == rat {
                    "1"
                } else {
                    "0"
                }
            };
            Ok(json!({"network_prefer":{"3G":flag("WCDMA"),"4G":flag("LTE"),"5G":flag("NR5G")}}))
        }
    }
    #[test]
    fn independent_shared_and_native_use_confirmed_state_once() -> Result<()> {
        for transport in [
            json!({"transport":"dedicated","channelReady":true,"identityVerified":true}),
            json!({"shared":true}),
            json!({"qmodemNetworkMode":true}),
        ] {
            let mut row =
                json!({"id":"2_1","model":"rm520f-gl","port":"/dev/ttyUSB2","instance":"1"});
            for (key, value) in transport.as_object().ok_or_else(unsupported)? {
                row[key] = value.clone()
            }
            let source = Arc::new(Mock {
                row: Mutex::new(row),
                mode: Mutex::new("AUTO".into()),
                writes: Mutex::default(),
                update: AtomicBool::new(true),
            });
            let controls = RadioControls::new(
                source.clone(),
                Arc::new(Mutex::new(Security::default())),
                Arc::new(AtomicBool::new(true)),
                Instant::now(),
            );
            let initial = controls.network_mode_read("2_1")?;
            assert_eq!(initial["mode"], "auto");
            let rev = initial["revision"].as_str().ok_or_else(unverified)?;
            assert!(
                controls
                    .network_mode_prepare("2_1", "AT+CFUN=0", rev, "user")
                    .is_err()
            );
            let c = controls.network_mode_prepare("2_1", "4g", rev, "user")?;
            let nonce = c["confirmation"].as_str().ok_or_else(unverified)?;
            let result = controls.network_mode_execute("2_1", "4g", rev, nonce, "user")?;
            assert_eq!(result["verified"], true);
            assert_eq!(result["mode"], "4g");
            assert!(
                controls
                    .network_mode_execute("2_1", "4g", rev, nonce, "user")
                    .is_err()
            );
            assert_eq!(source.writes.lock().map_err(|_| busy())?.len(), 1);
            assert!(
                controls
                    .network_mode_prepare("2_1", "auto", rev, "user")
                    .is_err()
            );
            let current = controls.network_mode_read("2_1")?;
            let rev = current["revision"].as_str().ok_or_else(unverified)?;
            let c = controls.network_mode_prepare("2_1", "auto", rev, "user")?;
            let nonce = c["confirmation"].as_str().ok_or_else(unverified)?;
            source.update.store(false, Ordering::Release);
            assert_eq!(
                controls
                    .network_mode_execute("2_1", "auto", rev, nonce, "user")
                    .err()
                    .map(|e| e.code),
                Some("NETWORK_MODE_UNKNOWN")
            );
            assert_eq!(source.writes.lock().map_err(|_| busy())?.len(), 2);
            let c = controls.network_mode_prepare("2_1", "auto", rev, "user")?;
            source.row.lock().map_err(|_| busy())?["instance"] = json!("2");
            assert_eq!(
                controls
                    .network_mode_execute(
                        "2_1",
                        "auto",
                        rev,
                        c["confirmation"].as_str().ok_or_else(unverified)?,
                        "user"
                    )
                    .err()
                    .map(|e| e.code),
                Some("NETWORK_MODE_CHANGED")
            );
            assert_eq!(source.writes.lock().map_err(|_| busy())?.len(), 2);
        }
        assert!(native(&json!({"network_prefer":{"3G":"0","4G":"0","5G":"0"}})).is_err());
        assert_eq!(
            native(&json!({"network_prefer":{"3G":"0","4G":"1","5G":"1"}}))?.mode,
            "4g+5g"
        );
        Ok(())
    }
}
