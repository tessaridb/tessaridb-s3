//! The wait before another attempt at a write the store answered `retry`: the store does not wait for the winner, so
//! losers that try again at once meet each other again.

use std::time::Duration;

/// `attempt` steps of `step_ms`, plus a random spread below one step so writers that collided once do not retry in
/// lockstep.
pub(crate) fn backoff(attempt: u32, step_ms: u64) -> Duration {
    let step = u64::from(attempt).saturating_mul(step_ms);
    Duration::from_millis(step.saturating_add(jitter(step)))
}

/// A random value in `0..bound` from the standard library's randomly keyed hasher; no dependency needed for a
/// back-off spread.
fn jitter(bound: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(bound);
    hasher.finish().checked_rem(bound).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::backoff;

    #[test]
    fn a_wait_grows_with_the_attempt_and_stays_under_twice_its_step() {
        for attempt in 1..=8 {
            let floor = Duration::from_millis(u64::from(attempt) * 20);
            let wait = backoff(attempt, 20);
            assert!(
                wait >= floor && wait < floor * 2,
                "attempt {attempt}: {wait:?}"
            );
        }
    }
}
