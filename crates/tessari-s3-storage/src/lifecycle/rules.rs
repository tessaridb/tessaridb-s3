//! When a rule's period has run out (S3: the day count from the event, rounded up to the next day boundary — next
//! midnight UTC).

use tessari_s3_core::lifecycle::{Expiration, Rule};
use tessari_s3_types::Timestamp;

/// Seconds in a lifecycle day.
const DAY_SECS: i64 = 86_400;

/// Whether `days` days after `since`, rounded up to the next day boundary, is not after `now`.
pub(super) fn elapsed(since: Timestamp, days: u32, now: Timestamp) -> bool {
    let Some(end) = i64::from(days)
        .checked_mul(DAY_SECS)
        .and_then(|span| since.seconds.checked_add(span))
    else {
        return false;
    };
    // Rounded up: an end exactly on a boundary stays; anything later moves to the next one.
    let rounded = if end.rem_euclid(DAY_SECS) == 0 && since.nanos == 0 {
        Some(end)
    } else {
        end.div_euclid(DAY_SECS)
            .checked_add(1)
            .and_then(|day| day.checked_mul(DAY_SECS))
    };
    rounded.is_some_and(|due| due <= now.seconds)
}

/// Whether `rule` expires a current version written at `modified`.
pub(super) fn current_expires(rule: &Rule, modified: Timestamp, now: Timestamp) -> bool {
    match &rule.expiration {
        Some(Expiration::Days(days)) => elapsed(modified, *days, now),
        Some(Expiration::Date(at)) => *at <= now,
        None => false,
    }
}

/// Whether `rule` removes a delete marker written at `modified` that is its key's only version: said outright, or
/// implied by an `Expiration` in days once the marker is that old.
pub(super) fn lone_marker_goes(rule: &Rule, modified: Timestamp, now: Timestamp) -> bool {
    rule.expired_object_delete_marker
        || matches!(rule.expiration, Some(Expiration::Days(days)) if elapsed(modified, days, now))
}

#[cfg(test)]
mod tests {
    use tessari_s3_types::Timestamp;

    use super::elapsed;

    const fn at(seconds: i64) -> Timestamp {
        Timestamp { seconds, nanos: 0 }
    }

    #[test]
    fn a_period_ends_at_the_day_boundary_after_it_runs_out() {
        // Written at 10:00 on day 0, one day: due at midnight starting day 2.
        let written = at(36_000);
        assert!(!elapsed(written, 1, at(2 * 86_400 - 1)));
        assert!(elapsed(written, 1, at(2 * 86_400)));
        // Written exactly at midnight: due exactly one day later.
        assert!(elapsed(at(86_400), 1, at(2 * 86_400)));
        assert!(!elapsed(at(86_400), 1, at(2 * 86_400 - 1)));
    }
}
