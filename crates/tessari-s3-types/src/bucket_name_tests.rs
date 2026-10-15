use super::BucketName;
use crate::{Error, ErrorCategory};

fn reason(name: &str) -> &'static str {
    match BucketName::new(name) {
        Err(Error::InvalidBucketName { reason }) => reason,
        other => panic!("{name:?} should be refused as an invalid bucket name, got {other:?}"),
    }
}

#[test]
fn names_that_follow_every_rule_are_accepted() {
    for name in [
        "abc",
        "my-bucket",
        "my.example.s3.bucket",
        "a1b2c3",
        "plan",
        &"a".repeat(63),
    ] {
        assert_eq!(
            BucketName::new(name).map(|b| b.as_str().to_owned()),
            Ok(name.to_owned()),
            "{name}"
        );
    }
}

#[test]
fn length_outside_three_to_sixty_three_is_refused() {
    assert_eq!(reason("ab"), "must be 3 to 63 characters long");
    assert_eq!(reason(&"a".repeat(64)), "must be 3 to 63 characters long");
}

#[test]
fn only_lowercase_letters_digits_periods_and_hyphens_are_allowed() {
    for name in ["Abc", "a_b_c", "abc d", "abç"] {
        assert_eq!(
            reason(name),
            "may contain only lowercase letters, digits, periods and hyphens",
            "{name}"
        );
    }
}

#[test]
fn first_and_last_character_must_be_a_letter_or_digit() {
    for name in ["-abc", ".abc", "abc-", "abc."] {
        assert_eq!(
            reason(name),
            "must begin and end with a letter or a digit",
            "{name}"
        );
    }
}

#[test]
fn two_adjacent_periods_are_refused() {
    assert_eq!(
        reason("example..com"),
        "must not contain two adjacent periods"
    );
}

#[test]
fn a_name_formatted_as_an_ip_address_is_refused() {
    assert_eq!(
        reason("192.168.5.4"),
        "must not be formatted as an IP address"
    );
    assert!(
        BucketName::new("192.168.5.4a").is_ok(),
        "only the dotted-quad shape is an address"
    );
}

#[test]
fn reserved_prefixes_and_suffixes_are_refused() {
    for name in ["xn--abc", "sthree-abc", "amzn-s3-demo-bucket"] {
        assert_eq!(
            reason(name),
            "must not start with a reserved prefix",
            "{name}"
        );
    }
    for name in [
        "abc-s3alias",
        "abc--ol-s3",
        "abc.mrap",
        "abc--x-s3",
        "abc--table-s3",
        "abc-an",
    ] {
        assert_eq!(
            reason(name),
            "must not end with a reserved suffix",
            "{name}"
        );
    }
}

#[test]
fn a_refusal_is_a_validation_failure() {
    assert_eq!(
        BucketName::new("ab").map_err(|e| e.category()),
        Err(ErrorCategory::Validation)
    );
}
