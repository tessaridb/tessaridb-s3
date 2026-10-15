//! Pacing a background copy to a budget of bytes a second, counted from the start of the copy, so it spends the
//! budget evenly instead of in bursts against the requests being served.

use std::num::{NonZeroU64, NonZeroU128};
use std::time::Duration;

use tokio::time::Instant;

/// A copy's budget and what it has spent.
pub(super) struct Pace {
    budget: NonZeroU64,
    started: Instant,
    sent: u64,
}

impl Pace {
    /// A pace of `budget` bytes a second, starting now.
    pub(super) fn new(budget: NonZeroU64) -> Self {
        Self {
            budget,
            started: Instant::now(),
            sent: 0,
        }
    }

    /// Counts `bytes` as sent, then waits until the copy is back within its budget.
    pub(super) async fn spend(&mut self, bytes: u64) {
        self.sent = self.sent.saturating_add(bytes);
        let wait = owed(self.sent, self.budget, self.started.elapsed());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// How much longer a copy that sent `sent` bytes `elapsed` after it began waits to stay within `budget` a second.
fn owed(sent: u64, budget: NonZeroU64, elapsed: Duration) -> Duration {
    // Nanoseconds the bytes sent are worth at the budget; u128 holds u64::MAX × 10⁹ without overflow.
    // A non-zero divisor: the division cannot fail.
    let due = u128::from(sent).saturating_mul(1_000_000_000) / NonZeroU128::from(budget);
    Duration::from_nanos(u64::try_from(due).unwrap_or(u64::MAX)).saturating_sub(elapsed)
}

#[cfg(test)]
#[path = "pace_tests.rs"]
mod tests;
