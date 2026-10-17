//! Temporary credentials against a real node: a session key signs only with its own session token — in the header,
//! a presigned URL or a form — and only until it expires; a token on any other key is refused; a session key never
//! opens the console; and disabling the user, unlinking its identity or a provider logout ends the session once its
//! resolution is older than the 5-second window, measured on a clock the test moves.

use std::sync::atomic::{AtomicI64, Ordering};

use axum::body::Body;
use axum::http::Request;
use tessari_s3_api::ApiState;
use tessari_s3_core::auth::{
    SecretKey, compose_session_token, post_signature, session_token_digest,
};
use tessari_s3_core::authz::{Role, UserName};
use tessari_s3_storage::Storage;
use tessari_s3_storage::users::NewSession;

use crate::access::{Key, user_with_key};
use crate::console_users::{sign_in_as, states};
use crate::post_object::{DATE, closing, form, policy, post};
use crate::{ACCESS_KEY, IGNORED, SECRET, Seen, amz_now_for_tests, call_as, send, signer};

const ISSUER: &str = "https://id.example.test";
const SUBJECT: &str = "subject-ann";
const HOUR: i64 = 3600;

fn after(at: i64, secs: i64) -> i64 {
    at.checked_add(secs).expect("an instant in range")
}

fn ann() -> UserName {
    UserName::new("ann").expect("a user name")
}

/// Temporary credentials for `ann` expiring at `expires`, their token drawn from `seed`.
async fn session(storage: &Storage, expires: i64, seed: u8) -> (Key, String) {
    let token = compose_session_token(expires, &[seed; 32]);
    let new = NewSession {
        expires,
        token_digest: session_token_digest(&token),
        issuer: ISSUER.to_owned(),
        subject: SUBJECT.to_owned(),
        policy: None,
    };
    let issued = storage
        .users()
        .issue_session(&ann(), &new)
        .await
        .expect("issued")
        .expect("ann exists");
    let key = Key {
        id: issued.access_key_id,
        secret: issued.secret.expose().to_owned(),
    };
    (key, token)
}

/// `ann` with a long-term key, its identity linked, owning bucket `own` with one object.
async fn prepared_ann(storage: &Storage, state: &ApiState) -> Key {
    let long_term = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    storage
        .identities()
        .link(&ann(), ISSUER, SUBJECT)
        .await
        .expect("linked");
    for (path, body) in [("/own", &b""[..]), ("/own/a.txt", &b"hello"[..])] {
        let made = call_as(state, long_term.pair(), "PUT", path, vec![], vec![], body).await;
        assert_eq!(made.status, 200, "{path}: {}", made.body);
    }
    long_term
}

/// GET `/own/a.txt` signed with `key`, carrying `token` in the header when given.
async fn read(state: &ApiState, key: (&str, &str), token: Option<&str>) -> (u16, Option<String>) {
    let headers = token
        .map(|token| vec![("x-amz-security-token", token)])
        .unwrap_or_default();
    let seen = call_as(state, key, "GET", "/own/a.txt", vec![], headers, b"").await;
    (seen.status, seen.code)
}

async fn presigned(state: &ApiState, key: &Key, token: Option<&str>) -> Seen {
    let amz_date = amz_now_for_tests();
    let unsigned = signer::Unsigned {
        method: "GET",
        host: "localhost:9100",
        path: "/own/a.txt",
        query: token
            .map(|token| vec![("X-Amz-Security-Token", Some(token))])
            .unwrap_or_default(),
        headers: vec![],
        access_key: &key.id,
        secret: &key.secret,
        region: state.region(),
        amz_date: &amz_date,
    };
    let request = Request::builder()
        .method("GET")
        .uri(signer::presign(&unsigned, 300))
        .header("host", "localhost:9100")
        .body(Body::empty())
        .expect("request");
    send(state, request).await
}

/// A form uploading `form.txt` into `own`, signed with `key` and naming `token` when given; the policy covers the
/// token field, as it must every field.
async fn form_upload(state: &ApiState, key: &Key, token: Option<&str>) -> Seen {
    let region = state.region();
    let credential = format!("{}/{}/{region}/s3/aws4_request", key.id, &DATE[..8]);
    let token_condition = token
        .map(|token| format!(r#",{{"x-amz-security-token":"{token}"}}"#))
        .unwrap_or_default();
    let encoded = policy(&format!(
        r#"{{"bucket":"own"}},{{"key":"form.txt"}},{{"x-amz-algorithm":"AWS4-HMAC-SHA256"}},{{"x-amz-credential":"{credential}"}},{{"x-amz-date":"{DATE}"}}{token_condition}"#
    ));
    let signature = post_signature(
        &SecretKey::new(key.secret.clone()),
        &DATE[..8],
        region,
        &encoded,
    )
    .expect("signed");
    let mut fields = vec![
        ("key".to_owned(), "form.txt".to_owned()),
        ("policy".to_owned(), encoded),
        ("x-amz-algorithm".to_owned(), "AWS4-HMAC-SHA256".to_owned()),
        ("x-amz-credential".to_owned(), credential),
        ("x-amz-date".to_owned(), DATE.to_owned()),
        ("x-amz-signature".to_owned(), signature),
    ];
    if let Some(token) = token {
        fields.push(("x-amz-security-token".to_owned(), token.to_owned()));
    }
    post(state, "own", form(&fields, "f", b"from a form", &closing())).await
}

fn status_and_code(seen: &Seen) -> (u16, Option<&str>) {
    (seen.status, seen.code.as_deref())
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn temporary_credentials_sign_only_with_their_own_token_and_until_they_expire() {
    let _why = IGNORED;
    let (state, console, clock) = states().await;
    let storage = state.storage().clone();
    let long_term = prepared_ann(&storage, &state).await;
    let now = clock.load(Ordering::Acquire);
    let (temporary, token) = session(&storage, after(now, HOUR), 7).await;
    let (_, other_token) = session(&storage, after(now, HOUR), 8).await;

    assert_eq!(
        read(&state, temporary.pair(), Some(&token)).await,
        (200, None)
    );
    for (why, key, presented) in [
        ("no token", temporary.pair(), None),
        (
            "another session's token",
            temporary.pair(),
            Some(other_token.as_str()),
        ),
        (
            "a token on a long-term key",
            long_term.pair(),
            Some(token.as_str()),
        ),
        (
            "a token on the root credential",
            (ACCESS_KEY, SECRET),
            Some(token.as_str()),
        ),
    ] {
        assert_eq!(
            read(&state, key, presented).await,
            (400, Some("InvalidToken".to_owned())),
            "{why}"
        );
    }

    assert_eq!(
        presigned(&state, &temporary, Some(&token)).await.status,
        200
    );
    assert_eq!(
        status_and_code(&presigned(&state, &temporary, None).await),
        (400, Some("InvalidToken"))
    );

    assert_eq!(
        form_upload(&state, &temporary, Some(&token)).await.status,
        204
    );
    assert_eq!(
        status_and_code(&form_upload(&state, &temporary, None).await),
        (400, Some("InvalidToken"))
    );

    assert_eq!(sign_in_as(&console, &temporary).await, Err(401));

    clock.store(after(now, HOUR), Ordering::Release);
    assert_eq!(
        read(&state, temporary.pair(), Some(&token)).await,
        (400, Some("ExpiredToken".to_owned()))
    );
    // A key the store no longer holds: its token still tells an expiry from a key that never existed.
    let gone = ("TSNOSUCHKEYAAAAAAAAA", temporary.secret.as_str());
    let stale = compose_session_token(now, &[9; 32]);
    assert_eq!(
        read(&state, gone, Some(&stale)).await,
        (400, Some("ExpiredToken".to_owned()))
    );
    let fresh = compose_session_token(after(now, 2 * HOUR), &[9; 32]);
    assert_eq!(
        read(&state, gone, Some(&fresh)).await,
        (403, Some("InvalidAccessKeyId".to_owned()))
    );
}

/// A session resolved at the clock's current time keeps working at +4 s after `end` and is refused from +5 s.
async fn ends_within_the_window<F, Fut>(state: &ApiState, clock: &AtomicI64, seed: u8, end: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    let start = clock.load(Ordering::Acquire);
    let (temporary, token) = session(state.storage(), after(start, HOUR), seed).await;
    assert_eq!(
        read(state, temporary.pair(), Some(&token)).await,
        (200, None)
    );
    end().await;
    clock.store(after(start, 4), Ordering::Release);
    assert_eq!(
        read(state, temporary.pair(), Some(&token)).await,
        (200, None)
    );
    clock.store(after(start, 5), Ordering::Release);
    assert_eq!(
        read(state, temporary.pair(), Some(&token)).await,
        (403, Some("InvalidAccessKeyId".to_owned()))
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_session_ends_with_its_identity_its_user_or_a_provider_logout() {
    let _why = IGNORED;
    let (state, _console, clock) = states().await;
    let storage = state.storage().clone();
    prepared_ann(&storage, &state).await;
    let identities = storage.identities();

    ends_within_the_window(&state, &clock, 1, || async {
        assert!(identities.unlink(&ann()).await.expect("unlinked"));
    })
    .await;
    identities
        .link(&ann(), ISSUER, SUBJECT)
        .await
        .expect("relinked");

    ends_within_the_window(&state, &clock, 2, || async {
        assert!(
            storage
                .users()
                .set_disabled(&ann(), true)
                .await
                .expect("disabled")
        );
    })
    .await;
    assert!(
        storage
            .users()
            .set_disabled(&ann(), false)
            .await
            .expect("enabled")
    );

    ends_within_the_window(&state, &clock, 3, || async {
        // The provider's logout comes after the session was issued, by the store's own clock.
        let at = after(ApiState::system_clock()(), 1);
        identities.end_sessions(&ann(), at).await.expect("ended");
    })
    .await;
}
