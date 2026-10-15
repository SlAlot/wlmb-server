//! 有界异步操作回执；按会话隔离任务，停机等待在途操作收尾。
use crate::{PanelError, Result};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
struct Job {
    owner: String,
    at: Instant,
    pending: bool,
    value: Value,
}
#[derive(Clone, Default)]
pub struct Actions {
    queue: Arc<(Mutex<Queue>, Condvar)>,
}
#[derive(Default)]
struct Queue {
    jobs: HashMap<String, Job>,
    closed: bool,
}
impl Actions {
    pub fn shutdown(&self, limit: Duration) -> bool {
        let (lock, changed) = &*self.queue;
        let Ok(mut queue) = lock.lock() else {
            return false;
        };
        queue.closed = true;
        let deadline = Instant::now() + limit;
        while queue.jobs.values().any(|j| j.pending) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            match changed.wait_timeout(queue, left) {
                Ok((next, _)) => queue = next,
                Err(_) => return false,
            }
        }
        true
    }
    fn prune(jobs: &mut HashMap<String, Job>) {
        jobs.retain(|_, job| job.pending || job.at.elapsed() < Duration::from_secs(120));
    }
    pub fn submit(
        &self,
        owner: &str,
        action: impl FnOnce() -> Result<Value> + Send + 'static,
    ) -> Result<Value> {
        if owner.is_empty() || owner.len() > 128 {
            return Err(PanelError::new("INVALID_REQUEST", 400));
        }
        let mut raw = [0; 24];
        getrandom::fill(&mut raw).map_err(|_| PanelError::new("RANDOM_UNAVAILABLE", 503))?;
        let key = raw.iter().map(|b| format!("{b:02x}")).collect::<String>();
        {
            let mut queue = self
                .queue
                .0
                .lock()
                .map_err(|_| PanelError::new("CONTROL_BUSY", 503))?;
            if queue.closed {
                return Err(PanelError::new("SERVICE_STOPPING", 503));
            }
            let jobs = &mut queue.jobs;
            Self::prune(jobs);
            if jobs.len() >= 16 || jobs.values().filter(|j| j.pending).count() >= 2 {
                return Err(PanelError::new("CONTROL_BUSY", 409));
            }
            jobs.insert(
                key.clone(),
                Job {
                    owner: owner.to_owned(),
                    at: Instant::now(),
                    pending: true,
                    value: json!({"state":"pending"}),
                },
            );
        }
        let queue = self.queue.clone();
        let id = key.clone();
        let run = move || {
            let value = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(action)) {
                Ok(Ok(result)) => json!({"state":"done","result":result}),
                Ok(Err(e)) => {
                    json!({"state":"error","error":{"code":e.code,"message":format!("{}；请读取状态，不要重复提交",e.message())}})
                }
                Err(_) => {
                    json!({"state":"error","error":{"code":"CONTROL_UNKNOWN","message":"操作结果未确认；请读取状态，不要重复提交"}})
                }
            };
            if let Ok(mut state) = queue.0.lock() {
                if let Some(job) = state.jobs.get_mut(&id) {
                    job.value = value;
                    job.pending = false;
                    job.at = Instant::now();
                }
                queue.1.notify_all();
            }
        };
        if std::thread::Builder::new()
            .name("wlmb-control".into())
            .stack_size(256 * 1024)
            .spawn(run)
            .is_err()
        {
            if let Ok(mut queue) = self.queue.0.lock() {
                queue.jobs.remove(&key);
                self.queue.1.notify_all();
            }
            return Err(PanelError::new("CONTROL_BUSY", 503));
        }
        Ok(json!({"job":key,"state":"pending"}))
    }
    pub fn result(&self, owner: &str, key: &str) -> Result<Value> {
        let mut queue = self
            .queue
            .0
            .lock()
            .map_err(|_| PanelError::new("CONTROL_BUSY", 503))?;
        let jobs = &mut queue.jobs;
        Self::prune(jobs);
        jobs.get(key)
            .filter(|j| j.owner == owner)
            .map(|j| j.value.clone())
            .ok_or_else(|| PanelError::new("CONTROL_RECEIPT_EXPIRED", 404))
    }
}
