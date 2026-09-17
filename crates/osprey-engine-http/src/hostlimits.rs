//! Process-wide connection caps: per host and in total, shared by every running task.
//!
//! Each worker connection holds one permit from its host's semaphore and one from the global
//! one for as long as the connection is open. Caps come from settings and can change while
//! tasks run; semaphores cannot shrink while permits are out, so a reduction is applied
//! opportunistically (whatever is free now is removed, the rest when it becomes free).

use osprey_domain::{ErrorKind, TaskError};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Hard ceiling regardless of settings: more connections never help and hurt every server.
pub const ABSOLUTE_MAX_CONNECTIONS: usize = 64;

struct Entry {
    sem: Arc<Semaphore>,
    /// Permits the semaphore currently represents (may lag the desired cap while shrinking).
    cap: usize,
}

struct State {
    per_host_cap: usize,
    total_cap: usize,
    hosts: HashMap<String, Entry>,
    total: Entry,
}

pub struct HostLimits {
    state: Mutex<State>,
}

/// Held by a worker for the lifetime of one connection.
#[derive(Debug)]
pub struct HostPermit {
    _host: OwnedSemaphorePermit,
    _total: OwnedSemaphorePermit,
}

impl HostLimits {
    pub fn new(per_host: usize, total: usize) -> Self {
        let per_host = per_host.clamp(1, ABSOLUTE_MAX_CONNECTIONS);
        let total = total.clamp(1, ABSOLUTE_MAX_CONNECTIONS * 16);
        Self {
            state: Mutex::new(State {
                per_host_cap: per_host,
                total_cap: total,
                hosts: HashMap::new(),
                total: Entry {
                    sem: Arc::new(Semaphore::new(total)),
                    cap: total,
                },
            }),
        }
    }

    /// The registry shared by every engine instance in this process.
    pub fn global() -> Arc<HostLimits> {
        static GLOBAL: OnceLock<Arc<HostLimits>> = OnceLock::new();
        GLOBAL
            .get_or_init(|| Arc::new(HostLimits::new(16, 64)))
            .clone()
    }

    /// Apply new caps (idempotent; called at the start of every run with the settings snapshot).
    pub fn configure(&self, per_host: usize, total: usize) {
        let mut st = self.state.lock();
        st.per_host_cap = per_host.clamp(1, ABSOLUTE_MAX_CONNECTIONS);
        st.total_cap = total.clamp(1, ABSOLUTE_MAX_CONNECTIONS * 16);
        let total_cap = st.total_cap;
        reconcile(&mut st.total, total_cap);
        let per = st.per_host_cap;
        for e in st.hosts.values_mut() {
            reconcile(e, per);
        }
    }

    pub fn per_host_cap(&self) -> usize {
        self.state.lock().per_host_cap
    }

    pub fn total_cap(&self) -> usize {
        self.state.lock().total_cap
    }

    fn semaphores(&self, host: &str) -> (Arc<Semaphore>, Arc<Semaphore>) {
        let mut st = self.state.lock();
        let per = st.per_host_cap;
        let total_cap = st.total_cap;
        reconcile(&mut st.total, total_cap);
        let total = st.total.sem.clone();
        let key = host.to_ascii_lowercase();
        let e = st.hosts.entry(key).or_insert_with(|| Entry {
            sem: Arc::new(Semaphore::new(per)),
            cap: per,
        });
        reconcile(e, per);
        (e.sem.clone(), total)
    }

    /// Non-blocking: a permit if both the host and the global cap have room.
    pub fn try_acquire(&self, host: &str) -> Option<HostPermit> {
        let (h, t) = self.semaphores(host);
        let host_permit = h.try_acquire_owned().ok()?;
        let total_permit = t.try_acquire_owned().ok()?;
        Some(HostPermit {
            _host: host_permit,
            _total: total_permit,
        })
    }

    /// Wait for a permit (used for the first connection of a task so a busy process still
    /// makes progress on every task eventually). Callers select this against their stop token.
    pub async fn acquire(&self, host: &str) -> Result<HostPermit, TaskError> {
        let (h, t) = self.semaphores(host);
        let host_permit = h
            .acquire_owned()
            .await
            .map_err(|_| TaskError::new(ErrorKind::Internal, "host limiter closed"))?;
        let total_permit = t
            .acquire_owned()
            .await
            .map_err(|_| TaskError::new(ErrorKind::Internal, "total limiter closed"))?;
        Ok(HostPermit {
            _host: host_permit,
            _total: total_permit,
        })
    }

    /// Permits currently free for `host` (min of host and global availability).
    pub fn available(&self, host: &str) -> usize {
        let (h, t) = self.semaphores(host);
        h.available_permits().min(t.available_permits())
    }
}

/// Grow immediately; shrink by forgetting whatever is free right now.
fn reconcile(e: &mut Entry, desired: usize) {
    if e.cap < desired {
        e.sem.add_permits(desired - e.cap);
        e.cap = desired;
    } else if e.cap > desired {
        let want = (e.cap - desired).min(e.sem.available_permits());
        if want > 0 {
            if let Ok(p) = e.sem.try_acquire_many(want as u32) {
                p.forget();
                e.cap -= want;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_are_enforced_and_resized() {
        let l = HostLimits::new(2, 3);
        let a = l.try_acquire("h1").unwrap();
        let b = l.try_acquire("H1").unwrap();
        assert!(l.try_acquire("h1").is_none(), "per-host cap");
        let c = l.try_acquire("h2").unwrap();
        assert!(l.try_acquire("h2").is_none(), "total cap");
        drop(c);
        assert_eq!(l.available("h2"), 1);
        l.configure(3, 3);
        assert!(l.try_acquire("h1").is_some(), "grown host cap");
        drop(a);
        drop(b);
        l.configure(1, 1);
        assert_eq!(l.per_host_cap(), 1);
        assert_eq!(l.total_cap(), 1);
    }
}
