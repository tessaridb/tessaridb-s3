use super::{Session, SessionError, issue, verify};

const SECRET: &[u8] = b"root-secret-key-0123456789abcdef";
const NOW: i64 = 1_791_417_600;
const HOUR: i64 = 3_600;
const NONCE: [u8; 16] = [9; 16];

fn token() -> String {
    issue(SECRET, "AKIAEXAMPLE", NOW, HOUR, NONCE)
}

#[test]
fn a_session_verifies_until_it_expires_and_names_its_key() {
    let expected = Session {
        key_id: "AKIAEXAMPLE".to_owned(),
        expires: NOW + HOUR,
    };
    assert_eq!(verify(SECRET, &token(), NOW), Ok(expected.clone()));
    assert_eq!(verify(SECRET, &token(), NOW + HOUR - 1), Ok(expected));
    assert_eq!(
        verify(SECRET, &token(), NOW + HOUR),
        Err(SessionError::Expired)
    );
}

#[test]
fn another_secret_or_any_changed_byte_is_refused() {
    let good = token();
    assert_eq!(
        verify(b"another-root-secret-0123456789ab", &good, NOW),
        Err(SessionError::Signature)
    );
    // Flip one character of the payload and, separately, of the signature.
    let (version, rest) = good.split_once('.').expect("versioned");
    let (payload, mac) = rest.split_once('.').expect("two parts");
    let flip = |text: &str| {
        let mut chars: Vec<char> = text.chars().collect();
        let first = chars.first_mut().expect("not empty");
        *first = if *first == 'A' { 'B' } else { 'A' };
        chars.into_iter().collect::<String>()
    };
    for tampered in [
        format!("{version}.{}.{mac}", flip(payload)),
        format!("{version}.{payload}.{}", flip(mac)),
    ] {
        assert!(
            matches!(
                verify(SECRET, &tampered, NOW),
                Err(SessionError::Signature | SessionError::Malformed)
            ),
            "{tampered}"
        );
        assert_ne!(verify(SECRET, &tampered, NOW), verify(SECRET, &good, NOW));
    }
}

#[test]
fn anything_that_is_not_a_token_of_this_version_is_malformed() {
    for text in [
        "",
        "v1",
        "v1.",
        "v1..",
        "v2.abc.def",
        "v1.!!!.???",
        "v1.abc.def.ghi",
    ] {
        assert_eq!(
            verify(SECRET, text, NOW),
            Err(SessionError::Malformed),
            "{text}"
        );
    }
    // A genuine signature under another version label, and with a part appended: the version and the shape are
    // checked, not only the signature.
    let good = token();
    let relabelled = good.replacen("v1.", "v2.", 1);
    assert_eq!(
        verify(SECRET, &relabelled, NOW),
        Err(SessionError::Malformed)
    );
    assert_eq!(
        verify(SECRET, &format!("{good}.extra"), NOW),
        Err(SessionError::Malformed)
    );
}

#[test]
fn two_sessions_issued_in_the_same_second_differ_by_their_nonce() {
    assert_ne!(token(), issue(SECRET, "AKIAEXAMPLE", NOW, HOUR, [8; 16]));
}
