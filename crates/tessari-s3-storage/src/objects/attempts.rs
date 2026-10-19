//! The attempts of one write of a key that may lose to another writer of the same key (ADR-0010). A lost attempt is
//! planned again only after a wait: the commit it lost to becomes readable once its group is synced, and an attempt
//! made at once reads the key from before that commit and loses to it again. The waits are random up to a bound that
//! doubles, and the write gives up when its deadline or its attempts are spent.

use std::time::Duration;

use tessari_s3_constants::{
    VERSION_WRITE_ATTEMPTS, VERSION_WRITE_BACKOFF_FIRST_MS, VERSION_WRITE_BACKOFF_MAX_MS,
    VERSION_WRITE_DEADLINE_MS,
};
use tokio::time::Instant;

/// One write's attempts so far.
pub(super) struct Attempts {
    started: Instant,
    made: u32,
    wait: Duration,
}

impl Attempts {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            made: 0,
            wait: Duration::from_millis(VERSION_WRITE_BACKOFF_FIRST_MS),
        }
    }

    /// Whether another attempt may be made: the first at once, each later one after a wait; false once
    /// [`VERSION_WRITE_ATTEMPTS`] were made or [`VERSION_WRITE_DEADLINE_MS`] has passed.
    pub(super) async fn next(&mut self) -> bool {
        if self.made > 0 {
            if self.made >= VERSION_WRITE_ATTEMPTS
                || self.started.elapsed() >= Duration::from_millis(VERSION_WRITE_DEADLINE_MS)
            {
                return false;
            }
            tokio::time::sleep(jittered(self.wait)).await;
            self.wait = self
                .wait
                .saturating_mul(2)
                .min(Duration::from_millis(VERSION_WRITE_BACKOFF_MAX_MS));
        }
        self.made = self.made.saturating_add(1);
        true
    }
}

/// A time between half of `wait` and all of it, so writers that lost together do not all try again together. Without
/// randomness the whole wait is used.
fn jittered(wait: Duration) -> Duration {
    let half = wait.checked_div(2).unwrap_or(wait);
    let mut bytes = [0_u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        return wait;
    }
    let span = u64::try_from(half.as_nanos())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let drawn = u64::from_le_bytes(bytes).checked_rem(span).unwrap_or(0);
    half.saturating_add(Duration::from_nanos(drawn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn the_first_attempt_is_made_at_once_and_a_later_one_after_a_wait() {
        let mut attempts = Attempts::new();
        let started = Instant::now();
        assert!(attempts.next().await);
        assert_eq!(started.elapsed(), Duration::ZERO);
        assert!(attempts.next().await);
        assert!(started.elapsed() >= Duration::from_millis(VERSION_WRITE_BACKOFF_FIRST_MS) / 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_attempts_end_at_the_deadline_within_one_wait() {
        let mut attempts = Attempts::new();
        let started = Instant::now();
        let mut made = 0_u32;
        while attempts.next().await {
            made += 1;
        }
        let deadline = Duration::from_millis(VERSION_WRITE_DEADLINE_MS);
        assert!(started.elapsed() >= deadline, "{:?}", started.elapsed());
        assert!(
            started.elapsed() <= deadline + Duration::from_millis(VERSION_WRITE_BACKOFF_MAX_MS),
            "{:?}",
            started.elapsed()
        );
        assert!(made > 1 && made <= VERSION_WRITE_ATTEMPTS, "{made}");
    }

    #[test]
    fn a_jittered_wait_is_between_half_the_wait_and_all_of_it() {
        let wait = Duration::from_millis(VERSION_WRITE_BACKOFF_MAX_MS);
        for _ in 0..1000 {
            let got = jittered(wait);
            assert!(got >= wait / 2 && got <= wait, "{got:?}");
        }
    }
}
