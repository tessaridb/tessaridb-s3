use super::{Fingerprint, JoinToken, TokenLifetime};

const DIGITS: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn a_fingerprint_is_exactly_64_lowercase_hex_digits() {
    assert_eq!(
        Fingerprint::new(DIGITS).map(|f| f.as_str().to_owned()),
        Some(DIGITS.to_owned())
    );
    for bad in [
        &DIGITS[1..],
        &DIGITS.to_uppercase(),
        &format!("{}g", &DIGITS[1..]),
        "",
    ] {
        assert_eq!(Fingerprint::new(bad), None, "{bad}");
    }
}

#[test]
fn a_token_lifetime_lies_between_one_minute_and_one_day() {
    assert_eq!(TokenLifetime::new(59), None);
    assert_eq!(TokenLifetime::new(60).map(TokenLifetime::secs), Some(60));
    assert_eq!(
        TokenLifetime::new(86_400).map(TokenLifetime::secs),
        Some(86_400)
    );
    assert_eq!(TokenLifetime::new(86_401), None);
}

#[test]
fn a_join_token_never_prints_its_digits() {
    let token = JoinToken::new(DIGITS).expect("a token's shape");
    assert_eq!(token.expose(), DIGITS);
    let printed = format!("{token:?}");
    assert!(!printed.contains("0123"), "{printed}");
}
