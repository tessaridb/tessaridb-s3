use std::num::NonZeroU64;
use std::time::Duration;

use super::owed;

const MIB: u64 = 1024 * 1024;

fn budget(bytes: u64) -> NonZeroU64 {
    NonZeroU64::new(bytes).expect("a budget")
}

#[test]
fn a_copy_ahead_of_its_budget_waits_for_the_difference() {
    assert_eq!(
        owed(32 * MIB, budget(32 * MIB), Duration::ZERO),
        Duration::from_secs(1)
    );
    assert_eq!(
        owed(32 * MIB, budget(32 * MIB), Duration::from_millis(250)),
        Duration::from_millis(750)
    );
}

#[test]
fn a_copy_within_its_budget_does_not_wait() {
    assert_eq!(
        owed(32 * MIB, budget(32 * MIB), Duration::from_secs(1)),
        Duration::ZERO
    );
    assert_eq!(
        owed(MIB, budget(32 * MIB), Duration::from_secs(5)),
        Duration::ZERO
    );
}

#[test]
fn the_largest_counts_neither_overflow_nor_wrap() {
    // Saturates at the longest wait a Duration of nanoseconds holds rather than wrapping to a short one.
    assert_eq!(
        owed(u64::MAX, budget(1), Duration::ZERO),
        Duration::from_nanos(u64::MAX)
    );
    assert_eq!(owed(MIB, budget(u64::MAX), Duration::ZERO), Duration::ZERO);
}
