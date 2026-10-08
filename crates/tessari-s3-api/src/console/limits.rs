//! A sliding one-minute window per key: sign-ins per client address, requests per signed-in key. The map is a
//! `std::sync::Mutex<HashMap>` rather than a concurrent map on purpose: the critical section is synchronous, never
//! crosses an `.await`, and is contended by a handful of operators. A poisoned lock refuses rather than letting
//! unmetered traffic through.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// The window, in seconds.
const WINDOW_SECS: i64 = 60;
/// Keys held before idle ones are dropped, so addresses that never come back do not accumulate.
const PRUNE_ABOVE: usize = 10_000;

/// Admissions per key within the last minute.
pub(crate) struct Window {
    limit: usize,
    seen: Mutex<HashMap<String, VecDeque<i64>>>,
}

impl Window {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Admits one request for `key` at `now` (seconds) and answers whether it was admitted; `Err` when the lock is
    /// poisoned, which the caller answers as a failure — never as an admission.
    pub(crate) fn admit(&self, key: &str, now: i64) -> Result<bool, ()> {
        let mut seen = self.seen.lock().map_err(|_| ())?;
        let floor = now.saturating_sub(WINDOW_SECS);
        if seen.len() > PRUNE_ABOVE {
            seen.retain(|_, times| times.back().is_some_and(|last| *last > floor));
        }
        let times = seen.entry(key.to_owned()).or_default();
        while times.front().is_some_and(|first| *first <= floor) {
            times.pop_front();
        }
        if times.len() >= self.limit {
            tracing::warn!(key, "console rate limit reached");
            return Ok(false);
        }
        times.push_back(now);
        Ok(true)
    }
}
