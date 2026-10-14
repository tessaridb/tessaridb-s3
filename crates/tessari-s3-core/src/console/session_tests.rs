use super::{Session, SessionError, Subject, issue, verify};

const SECRET: &[u8] = b"root-secret-key-0123456789abcdef";
const NOW: i64 = 1_791_417_600;
const HOUR: i64 = 3_600;
const NONCE: [u8; 16] = [9; 16];

fn key() -> Subject {
    Subject::Key("AKIAEXAMPLE".to_owned())
}

fn token() -> String {
    issue(SECRET, &key(), NOW, HOUR, Some(NOW), NONCE)
}

#[test]
fn a_session_verifies_until_it_expires_and_names_its_key() {
    let expected = Session {
        subject: key(),
        expires: NOW + HOUR,
        authenticated: Some(NOW),
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
        "v1.abc.def",
        "v2.abc.def",
        "v4.abc.def",
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
    // v2 is the format before the authentication time was carried; its tokens are refused, not read without it.
    let relabelled = good.replacen("v3.", "v2.", 1);
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
    assert_ne!(
        token(),
        issue(SECRET, &key(), NOW, HOUR, Some(NOW), [8; 16])
    );
}

#[test]
fn a_session_names_whether_it_holds_a_key_or_a_person() {
    let person = Subject::User("alice".to_owned());
    let session = verify(SECRET, &issue(SECRET, &person, NOW, HOUR, None, NONCE), NOW);
    assert_eq!(
        session,
        Ok(Session {
            subject: person,
            expires: NOW + HOUR,
            authenticated: None,
        })
    );
    // A key and a user spelled alike are two different sessions, and each limits under its own label.
    let alike = Subject::Key("alice".to_owned());
    assert_ne!(
        issue(SECRET, &alike, NOW, HOUR, None, NONCE),
        issue(
            SECRET,
            &Subject::User("alice".to_owned()),
            NOW,
            HOUR,
            None,
            NONCE
        )
    );
    assert_eq!(alike.label(), "key:alice");
    assert_eq!(Subject::User("alice".to_owned()).label(), "user:alice");
    // The action record names a key as it always has, and a person so that no generated key id can be mistaken for them.
    assert_eq!(alike.operator(), "alice");
    assert_eq!(Subject::User("alice".to_owned()).operator(), "user:alice");
}

#[test]
fn a_session_carries_when_its_holder_authenticated_and_answers_whether_that_is_recent() {
    let earlier = NOW - 200;
    let session = verify(
        SECRET,
        &issue(SECRET, &key(), NOW, HOUR, Some(earlier), NONCE),
        NOW,
    )
    .expect("verifies");
    assert_eq!(session.authenticated, Some(earlier));
    // Recent within the bound, inclusive; older, or unknown, is not.
    assert!(session.authenticated_within(earlier + 300, 300));
    assert!(!session.authenticated_within(earlier + 301, 300));
    let unknown =
        verify(SECRET, &issue(SECRET, &key(), NOW, HOUR, None, NONCE), NOW).expect("verifies");
    assert_eq!(unknown.authenticated, None);
    assert!(!unknown.authenticated_within(NOW, 300));
    // The time is signed: a token claiming another authentication time is another token.
    assert_ne!(
        issue(SECRET, &key(), NOW, HOUR, Some(NOW), NONCE),
        issue(SECRET, &key(), NOW, HOUR, Some(earlier), NONCE)
    );
}
