use crate::Error;
use crate::config::tests::{CREDENTIALS, load};

const HEX: &str = "5f1e0c2b9a8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f";

fn with_key(value: &str) -> Vec<(&str, &str)> {
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_IAM_KEY", value));
    vars
}

#[test]
fn without_the_variable_no_user_key_can_be_sealed() {
    assert!(load(&CREDENTIALS).expect("loads").iam_key.is_none());
}

#[test]
fn the_key_is_read_from_its_hex_digits() {
    let config = load(&with_key(HEX)).expect("loads");
    let key = config.iam_key.expect("a key");
    assert_eq!(key.expose().first(), Some(&0x5f));
    assert_eq!(key.expose().last(), Some(&0x2f));
}

#[test]
fn a_key_that_is_not_sixty_four_hex_digits_is_refused_without_echoing_it() {
    let secret = "not-a-key-but-a-secret-anyway";
    let error = load(&with_key(secret)).expect_err("refused");
    assert_eq!(
        error,
        Error::InvalidConfig {
            key: "TESSARIDB_S3_IAM_KEY",
            reason: "not 64 hexadecimal digits",
        }
    );
    assert!(!error.to_string().contains(secret));
}

#[test]
fn the_key_never_appears_in_the_configuration_debug_output() {
    let config = load(&with_key(HEX)).expect("loads");
    let shown = format!("{config:?}");
    assert!(shown.contains("iam_key: Some(IamKey(..))"), "{shown}");
    assert!(!shown.contains(HEX));
}
