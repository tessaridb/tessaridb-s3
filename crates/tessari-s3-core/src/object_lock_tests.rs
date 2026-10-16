use tessari_s3_types::Timestamp;

use super::{DefaultRetention, LegalHold, Mode, Period};

const DAY: i64 = 86_400;

fn at(seconds: i64) -> Timestamp {
    Timestamp { seconds, nanos: 0 }
}

#[test]
fn only_the_spellings_s3_uses_name_a_mode_or_a_hold() {
    for mode in [Mode::Compliance, Mode::Governance] {
        assert_eq!(Mode::parse(mode.as_str()), Some(mode));
    }
    for other in ["", "compliance", "Governance", "NONE", " GOVERNANCE"] {
        assert_eq!(Mode::parse(other), None, "{other:?}");
    }
    assert_eq!(LegalHold::parse("ON"), Some(LegalHold::On));
    assert_eq!(LegalHold::parse("OFF"), Some(LegalHold::Off));
    assert_eq!(LegalHold::parse("on"), None);
    assert_eq!(LegalHold::On.as_str(), "ON");
}

#[test]
fn a_default_period_is_whole_days_or_years_within_s3_limits() {
    assert!(DefaultRetention::new(Mode::Governance, Period::Days(1)).is_some());
    assert!(DefaultRetention::new(Mode::Governance, Period::Days(36_500)).is_some());
    assert!(DefaultRetention::new(Mode::Governance, Period::Years(100)).is_some());
    for period in [
        Period::Days(0),
        Period::Days(36_501),
        Period::Years(0),
        Period::Years(101),
    ] {
        assert!(
            DefaultRetention::new(Mode::Compliance, period).is_none(),
            "{period:?}"
        );
    }
}

#[test]
fn a_default_retains_from_the_write_for_its_period() {
    let days = DefaultRetention::new(Mode::Compliance, Period::Days(3)).expect("valid");
    assert_eq!(days.until(at(1_000)), Some(at(1_000 + 3 * DAY)));
    let years = DefaultRetention::new(Mode::Governance, Period::Years(2)).expect("valid");
    assert_eq!(years.until(at(0)), Some(at(2 * 365 * DAY)));
    assert_eq!(
        days.until(at(i64::MAX)),
        None,
        "overflow is refused, not wrapped"
    );
}
