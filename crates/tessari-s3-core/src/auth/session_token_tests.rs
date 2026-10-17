//! A session token: its expiry readable from the token alone, its digest what is stored, and a presented token
//! matched against that digest only when every byte agrees.

use super::session_token::{compose, digest, expiry, matches};

const RANDOM: [u8; 32] = [7; 32];

#[test]
fn a_token_carries_its_expiry_and_matches_only_its_own_digest() {
    let token = compose(1_792_000_000, &RANDOM);
    assert!(token.starts_with("TSS1"), "{token}");
    assert_eq!(expiry(&token), Some(1_792_000_000));
    let stored = digest(&token);
    assert!(matches(&token, &stored));
    let mut other = token.clone();
    other.push('A');
    assert!(!matches(&other, &stored), "one more character");
    let different = compose(1_792_000_000, &[8; 32]);
    assert!(
        !matches(&different, &stored),
        "another draw at the same expiry"
    );
    let later = compose(1_792_000_001, &RANDOM);
    assert!(!matches(&later, &stored), "the same draw at another expiry");
}

#[test]
fn a_token_that_is_not_one_of_ours_has_no_expiry() {
    for presented in ["", "TSS1", "TSS1!!!!", "FwoGZXIvYXdzEJr", "TSS2AAAA"] {
        assert_eq!(expiry(presented), None, "{presented}");
    }
}
