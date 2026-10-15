//! 单调计数差值与流量账本；接口换代或计数回绕不累加伪造流量。
use crate::{PanelError, Result};
use serde_json::{Value, json};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: u32,
    pub month: u32,
    pub day: u32,
}
fn bad() -> PanelError {
    PanelError::new("TRAFFIC_INVALID", 422)
}
impl Date {
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        if raw.len() != 10
            || !raw.is_ascii()
            || &raw[4..5] != "-"
            || &raw[7..8] != "-"
            || !raw
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
        {
            return Err(bad());
        }
        let year = raw[..4].parse().map_err(|_| bad())?;
        let month = raw[5..7].parse().map_err(|_| bad())?;
        let day = raw[8..].parse().map_err(|_| bad())?;
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            2 => {
                if leap {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => 0,
        };
        if !(2020..=2199).contains(&year) || day == 0 || day > days {
            return Err(bad());
        }
        Ok(Self { year, month, day })
    }
    pub fn text(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
    fn cycle(self, start: u32) -> (u32, u32) {
        if self.day >= start {
            (self.year, self.month)
        } else if self.month > 1 {
            (self.year, self.month - 1)
        } else {
            (self.year - 1, 12)
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budget {
    pub enabled: bool,
    pub limit: u64,
    pub start: u32,
    pub threshold: u32,
    pub cutoff: bool,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            enabled: false,
            limit: 0,
            start: 1,
            threshold: 90,
            cutoff: false,
        }
    }
}
impl Budget {
    pub fn decode(v: &Value) -> Result<Self> {
        let enabled = v["enabled"].as_bool().ok_or_else(bad)?;
        let limit = v["limitBytes"]
            .as_u64()
            .filter(|n| *n <= 9_007_199_254_740_991 && (!enabled || *n > 0))
            .ok_or_else(bad)?;
        let start = v["startDay"]
            .as_u64()
            .filter(|n| (1..=28).contains(n))
            .ok_or_else(bad)? as u32;
        let threshold = v["threshold"]
            .as_u64()
            .filter(|n| (1..=100).contains(n))
            .ok_or_else(bad)? as u32;
        let cutoff = v["cutoffEnabled"].as_bool().ok_or_else(bad)?;
        if !enabled && (limit != 0 || cutoff) {
            return Err(bad());
        }
        Ok(Self {
            enabled,
            limit,
            start,
            threshold,
            cutoff,
        })
    }
    pub fn public(&self, used: u64, ready: bool) -> Value {
        json!({"supported":true,"enabled":self.enabled,"limitBytes":self.limit,"startDay":self.start,"threshold":self.threshold,
            "cutoffEnabled":self.cutoff,"warning":ready && self.enabled && (used as u128)*100 >= (self.limit as u128)*(self.threshold as u128),
            "limitReached":ready && self.enabled && used>=self.limit})
    }
}
#[derive(Clone, Default)]
pub struct Accounting {
    pub budget: Budget,
    date: Option<Date>,
    today: u64,
    month: u64,
    baseline: Option<(String, u64, u64)>,
    clock_valid: bool,
    pub persistence_ok: bool,
    pub observed: bool,
}
impl Accounting {
    pub fn restore(raw: &[u8]) -> Result<Self> {
        if raw.len() > 4096 {
            return Err(bad());
        }
        let v: Value = serde_json::from_slice(raw).map_err(|_| bad())?;
        if v.get("version").is_none() && v["format"] == 1 {
            let date = Date::parse(v["day"].as_str().ok_or_else(bad)?)?;
            let period = Date::parse(v["month"].as_str().ok_or_else(bad)?)?;
            let today = v["todayBytes"].as_u64().ok_or_else(bad)?;
            let month = v["monthBytes"].as_u64().ok_or_else(bad)?;
            if period.day != 1
                || period.year != date.year
                || period.month != date.month
                || today > month
            {
                return Err(bad());
            }
            return Ok(Self {
                date: Some(date),
                today,
                month,
                persistence_ok: true,
                ..Self::default()
            });
        }
        if v["version"] != 1 {
            return Err(bad());
        }
        let date = if v["date"].is_null() {
            None
        } else {
            Some(Date::parse(v["date"].as_str().ok_or_else(bad)?)?)
        };
        let today = v["todayBytes"].as_u64().ok_or_else(bad)?;
        let month = v["monthBytes"].as_u64().ok_or_else(bad)?;
        Ok(Self {
            budget: Budget::decode(&v["budget"])?,
            date,
            today,
            month,
            persistence_ok: true,
            ..Self::default()
        })
    }
    pub fn checkpoint(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&json!({"version":1,"date":self.date.map(Date::text),"todayBytes":self.today,"monthBytes":self.month,
            "budget":self.budget.public(self.month,false)})).map_err(|_| bad())
    }
    pub fn configure(&mut self, next: Budget) {
        if self.budget.start != next.start {
            self.month = 0;
        }
        self.budget = next;
    }
    pub fn sample(&mut self, date: Option<Date>, counters: Option<(&str, u64, u64)>) -> Result<()> {
        self.clock_valid = date.is_some_and(|d| self.date.is_none_or(|old| d >= old));
        let mut delta = 0;
        if let Some((name, rx, tx)) = counters {
            if let Some((old_name, old_rx, old_tx)) = &self.baseline {
                if name == old_name {
                    if let (Some(down), Some(up)) =
                        (rx.checked_sub(*old_rx), tx.checked_sub(*old_tx))
                    {
                        delta = down.checked_add(up).ok_or_else(bad)?;
                    }
                }
            }
            self.baseline = Some((name.into(), rx, tx));
        } else {
            self.baseline = None;
        }
        if !self.clock_valid {
            return Ok(());
        }
        let d = date.ok_or_else(bad)?;
        if let Some(old) = self.date {
            if d.cycle(self.budget.start) != old.cycle(self.budget.start) {
                self.month = 0;
            }
            if d != old {
                self.today = 0;
            }
        }
        self.date = Some(d);
        self.today = self.today.checked_add(delta).ok_or_else(bad)?;
        self.month = self.month.checked_add(delta).ok_or_else(bad)?;
        self.observed = counters.is_some();
        Ok(())
    }
    pub fn public(&self) -> Value {
        json!({"todayBytes":if self.clock_valid && self.observed {Some(self.today)} else {None},
            "monthBytes":if self.clock_valid && self.observed {Some(self.month)} else {None},"date":self.date.map(Date::text),
            "source":"plugin-single-wan-counter-delta","persistenceOk":self.persistence_ok,"clockValid":self.clock_valid,
            "partialHistory":true,"boundaryAttribution":"sample-end"})
    }
    pub fn limit(&self) -> Value {
        self.budget
            .public(self.month, self.clock_valid && self.observed)
    }
}
