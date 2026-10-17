//! Temporary credentials from the STS endpoint, against a real node and the test identity provider: a person whose
//! identity is linked to a user presents an ID token for that user's role and gets a session key that signs with its
//! session token; every other request — a token wrong in any of the ways a token can be, a role that is not the
//! linked user's, an unlinked identity, a disabled user, a parameter out of its limits or not served here — is refused
//! in the STS error document with its own code, and issues nothing.

use axum::body::Body;
use axum::http::Request;
use tessari_s3_api::ApiState;
use tessari_s3_constants::CONSOLE_OIDC_REFRESH_MIN_SECS;
use tessari_s3_core::authz::{Role, UserName};
use tessari_s3_infrastructure::oidc::OidcClient;
use tessari_s3_infrastructure::{KeySignIn, OidcSettings};
use tessari_s3_types::SecretKey;

use crate::access::{Key, user_with_key};
use crate::oidc_provider::{CLIENT_ID, CLIENT_SECRET, Provider, REDIRECT, SUBJECT, Shape};
use crate::{IGNORED, Seen, call_as, fresh, live_config, prepared, scratch_dir, send, test_node};

const ROLE: &str = "arn:aws:iam::alpha:role/ann";

/// Why, the request's fields, and the status and code it is refused with.
type Refusal<'a> = (&'static str, Vec<(&'a str, &'a str)>, u16, &'static str);
const NAMESPACE: &str = "https://sts.amazonaws.com/doc/2011-06-15/";

fn settings(provider: &Provider) -> OidcSettings {
    OidcSettings {
        issuer: provider.issuer.clone(),
        client_id: CLIENT_ID.to_owned(),
        client_secret: SecretKey::new(CLIENT_SECRET.to_owned()),
        redirect_uri: REDIRECT.to_owned(),
        mfa_amr: vec!["mfa".to_owned()],
        break_glass_webhook: None,
        key_sign_in: KeySignIn::All,
    }
}

fn ann() -> UserName {
    UserName::new("ann").expect("a user name")
}

/// A node issuing temporary credentials through `provider`, at most `per_minute` requests an address; `ann` (space
/// `alpha`, linked to the provider's subject) owns bucket `own` with one object; `bob` is in `alpha` too.
async fn issuing(provider: &Provider, per_minute: usize) -> (ApiState, Key) {
    let config = live_config(test_node(), None, &scratch_dir());
    let storage = prepared(&config).await;
    let state = ApiState::new(&config, ApiState::system_clock(), storage.clone());
    let client =
        OidcClient::new(&settings(provider), CONSOLE_OIDC_REFRESH_MIN_SECS).expect("a client");
    assert!(state.accept_web_identity(client, per_minute));
    let long_term = user_with_key(&storage, "ann", "alpha", Role::Member, true).await;
    user_with_key(&storage, "bob", "alpha", Role::Member, false).await;
    storage
        .identities()
        .link(&ann(), &provider.issuer, SUBJECT)
        .await
        .expect("linked");
    for (path, body) in [("/own", &b""[..]), ("/own/a.txt", &b"hello"[..])] {
        let made = call_as(&state, long_term.pair(), "PUT", path, vec![], vec![], body).await;
        assert_eq!(made.status, 200, "{path}: {}", made.body);
    }
    (state, long_term)
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            b' ' => "+".to_owned(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// `POST /` with `fields` form-encoded, unsigned, as an STS client sends it.
async fn ask(state: &ApiState, fields: &[(&str, &str)]) -> Seen {
    let body: Vec<String> = fields
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect();
    let body = body.join("&");
    let request = Request::builder()
        .method("POST")
        .uri("/")
        .header("host", "localhost:9100")
        .header(
            "content-type",
            "application/x-www-form-urlencoded; charset=utf-8",
        )
        .header("content-length", body.len().to_string())
        .body(Body::from(body))
        .expect("request");
    send(state, request).await
}

/// The fields of a request for `ann`'s role presenting `token`, with `extra` added and `without` left out.
fn asking<'a>(
    token: &'a str,
    extra: &[(&'a str, &'a str)],
    without: &str,
) -> Vec<(&'a str, &'a str)> {
    let mut fields = vec![
        ("Action", "AssumeRoleWithWebIdentity"),
        ("Version", "2011-06-15"),
        ("RoleArn", ROLE),
        ("RoleSessionName", "laptop"),
        ("WebIdentityToken", token),
    ];
    fields.retain(|(name, _)| *name != without);
    fields.extend_from_slice(extra);
    fields
}

fn element<'a>(body: &'a str, name: &str) -> &'a str {
    body.split_once(&format!("<{name}>"))
        .and_then(|(_, rest)| rest.split_once(&format!("</{name}>")))
        .map(|(value, _)| value)
        .unwrap_or_else(|| panic!("no {name} in {body}"))
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_linked_person_gets_temporary_credentials_that_sign_with_their_token() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (state, _) = issuing(&provider, 100).await;
    let token = provider.identity_token(Shape::Good);
    let before = state.now();
    let answer = ask(&state, &asking(&token, &[("DurationSeconds", "900")], "")).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(
        answer
            .headers
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    assert!(answer.body.contains(&format!(
        "<AssumeRoleWithWebIdentityResponse xmlns=\"{NAMESPACE}\">"
    )));
    assert_eq!(
        element(&answer.body, "SubjectFromWebIdentityToken"),
        SUBJECT
    );
    assert_eq!(element(&answer.body, "Provider"), provider.issuer);
    assert_eq!(element(&answer.body, "Audience"), CLIENT_ID);
    assert_eq!(
        element(&answer.body, "Arn"),
        "arn:aws:sts::alpha:assumed-role/ann/laptop"
    );
    assert_eq!(element(&answer.body, "AssumedRoleId"), "ann:laptop");
    let expiration = element(&answer.body, "Expiration");
    let earliest = tessari_s3_types::Timestamp {
        seconds: before.checked_add(900).expect("in range"),
        nanos: 0,
    }
    .iso8601_millis();
    assert!(
        expiration >= earliest.as_str(),
        "{expiration} before {earliest}"
    );
    let temporary = Key {
        id: element(&answer.body, "AccessKeyId").to_owned(),
        secret: element(&answer.body, "SecretAccessKey").to_owned(),
    };
    let session_token = element(&answer.body, "SessionToken").to_owned();
    let read = call_as(
        &state,
        temporary.pair(),
        "GET",
        "/own/a.txt",
        vec![],
        vec![("x-amz-security-token", &session_token)],
        b"",
    )
    .await;
    assert_eq!((read.status, read.body.as_str()), (200, "hello"));
    let without = call_as(
        &state,
        temporary.pair(),
        "GET",
        "/own/a.txt",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(without.code.as_deref(), Some("InvalidToken"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_refused_request_names_its_reason_and_issues_nothing() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (state, _) = issuing(&provider, 100).await;
    let good = provider.identity_token(Shape::Good);
    let shaped = |shape| provider.identity_token(shape);
    let tokens: Vec<(Shape, String)> = [
        Shape::NoSecondFactor,
        Shape::WrongAudience,
        Shape::WrongIssuer,
        Shape::Expired,
        Shape::AlgNone,
        Shape::Hs256,
        Shape::SecondKey,
    ]
    .into_iter()
    .map(|shape| (shape, shaped(shape)))
    .collect();
    let keys = || async {
        state
            .storage()
            .users()
            .keys_of(&ann())
            .await
            .expect("keys")
            .len()
    };
    let held = keys().await;
    for (shape, token) in &tokens {
        let expected = match shape {
            Shape::NoSecondFactor => (403, "IDPRejectedClaim"),
            Shape::Expired => (400, "ExpiredTokenException"),
            _ => (400, "InvalidIdentityToken"),
        };
        let answer = ask(&state, &asking(token, &[], "")).await;
        assert_eq!(
            (answer.status, answer.code.as_deref()),
            (expected.0, Some(expected.1)),
            "{shape:?}"
        );
        assert!(answer.body.contains(&format!(
            "<ErrorResponse xmlns=\"{NAMESPACE}\"><Error><Type>Sender</Type>"
        )));
    }
    let malformed: [Refusal<'_>; 12] = [
        (
            "not a token",
            asking("not.a.token", &[], ""),
            400,
            "InvalidIdentityToken",
        ),
        (
            "another user's role",
            {
                let mut fields = asking(&good, &[], "RoleArn");
                fields.push(("RoleArn", "arn:aws:iam::alpha:role/bob"));
                fields
            },
            403,
            "AccessDenied",
        ),
        (
            "the role in another space",
            {
                let mut fields = asking(&good, &[], "RoleArn");
                fields.push(("RoleArn", "arn:aws:iam::beta:role/ann"));
                fields
            },
            403,
            "AccessDenied",
        ),
        (
            "too short",
            asking(&good, &[("DurationSeconds", "899")], ""),
            400,
            "ValidationError",
        ),
        (
            "too long",
            asking(&good, &[("DurationSeconds", "43201")], ""),
            400,
            "ValidationError",
        ),
        (
            "a one-letter session name",
            {
                let mut fields = asking(&good, &[], "RoleSessionName");
                fields.push(("RoleSessionName", "a"));
                fields
            },
            400,
            "ValidationError",
        ),
        (
            "a session name with a space",
            {
                let mut fields = asking(&good, &[], "RoleSessionName");
                fields.push(("RoleSessionName", "my laptop"));
                fields
            },
            400,
            "ValidationError",
        ),
        (
            "no token",
            asking(&good, &[], "WebIdentityToken"),
            400,
            "ValidationError",
        ),
        (
            "a session policy",
            asking(&good, &[("Policy", "{}")], ""),
            400,
            "MalformedPolicyDocument",
        ),
        (
            "another action",
            {
                let mut fields = asking(&good, &[], "Action");
                fields.push(("Action", "AssumeRole"));
                fields
            },
            400,
            "InvalidAction",
        ),
        (
            "a parameter not served",
            asking(&good, &[("ProviderId", "www.example.com")], ""),
            400,
            "ValidationError",
        ),
        (
            "a repeated parameter",
            asking(&good, &[("RoleSessionName", "again")], ""),
            400,
            "ValidationError",
        ),
    ];
    for (why, fields, status, code) in &malformed {
        let answer = ask(&state, fields).await;
        assert_eq!(
            (answer.status, answer.code.as_deref()),
            (*status, Some(*code)),
            "{why}: {}",
            answer.body
        );
    }
    let users = state.storage().users();
    assert!(users.set_disabled(&ann(), true).await.expect("disabled"));
    let disabled = ask(&state, &asking(&good, &[], "")).await;
    assert_eq!(
        (disabled.status, disabled.code.as_deref()),
        (403, Some("IDPRejectedClaim"))
    );
    assert!(users.set_disabled(&ann(), false).await.expect("enabled"));
    assert!(
        state
            .storage()
            .identities()
            .unlink(&ann())
            .await
            .expect("unlinked")
    );
    let unlinked = ask(&state, &asking(&good, &[], "")).await;
    assert_eq!(
        (unlinked.status, unlinked.code.as_deref()),
        (403, Some("IDPRejectedClaim"))
    );
    assert_eq!(keys().await, held, "a refusal issued a key");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn without_a_provider_nothing_is_issued_and_asking_is_metered_per_address() {
    let _why = IGNORED;
    let (bare, _pool) = fresh().await;
    let refused = ask(&bare, &asking("a.b.c", &[], "")).await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (501, Some("NotImplemented"))
    );
    let provider = Provider::start().await;
    let (state, _) = issuing(&provider, 2).await;
    let good = provider.identity_token(Shape::Good);
    for _ in 0..2 {
        assert_eq!(ask(&state, &asking(&good, &[], "")).await.status, 200);
    }
    let throttled = ask(&state, &asking(&good, &[], "")).await;
    assert_eq!(
        (throttled.status, throttled.code.as_deref()),
        (400, Some("Throttling"))
    );
}

/// Temporary credentials for `ann` narrowed by `policy`, as the key and its session token.
async fn narrowed(state: &ApiState, provider: &Provider, policy: &str) -> (Key, String) {
    let token = provider.identity_token(Shape::Good);
    let answer = ask(state, &asking(&token, &[("Policy", policy)], "")).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let key = Key {
        id: element(&answer.body, "AccessKeyId").to_owned(),
        secret: element(&answer.body, "SecretAccessKey").to_owned(),
    };
    (key, element(&answer.body, "SessionToken").to_owned())
}

async fn as_session(
    state: &ApiState,
    session: &(Key, String),
    method: &str,
    path: &str,
) -> (u16, Option<String>) {
    let seen = call_as(
        state,
        session.0.pair(),
        method,
        path,
        vec![],
        vec![("x-amz-security-token", &session.1)],
        b"data",
    )
    .await;
    (seen.status, seen.code)
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_session_policy_narrows_what_its_user_may_do_and_never_widens_it() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (state, _) = issuing(&provider, 100).await;
    assert_eq!(
        crate::call(&state, "PUT", "/foreign", vec![], b"")
            .await
            .status,
        200
    );
    let denied = (403, Some("AccessDenied".to_owned()));
    let reader = narrowed(
        &state,
        &provider,
        r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:GetObject","Resource":"arn:aws:s3:::own/*"}]}"#,
    )
    .await;
    assert_eq!(
        as_session(&state, &reader, "GET", "/own/a.txt").await,
        (200, None)
    );
    assert_eq!(
        as_session(&state, &reader, "PUT", "/own/b.txt").await,
        denied,
        "ann may write her bucket; the session policy does not allow it"
    );
    assert_eq!(
        as_session(&state, &reader, "GET", "/").await,
        denied,
        "no ListAllMyBuckets"
    );
    let fenced = narrowed(
        &state,
        &provider,
        r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:*","Resource":"*"},{"Effect":"Deny","Action":"s3:GetObject","Resource":"arn:aws:s3:::own/a.*"}]}"#,
    )
    .await;
    assert_eq!(
        as_session(&state, &fenced, "GET", "/own/a.txt").await,
        denied,
        "the Deny outweighs"
    );
    assert_eq!(
        as_session(&state, &fenced, "PUT", "/own/b.txt").await,
        (200, None)
    );
    assert_eq!(
        as_session(&state, &fenced, "GET", "/own/b.txt").await,
        (200, None)
    );
    assert_eq!(
        as_session(&state, &fenced, "GET", "/foreign/x").await,
        denied,
        "a policy allowing everything grants nothing the user lacks"
    );
    // A batch delete is asked key by key of the session policy, as of a bucket's: one entry refused, the other done.
    let keeper = narrowed(
        &state,
        &provider,
        r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:*","Resource":"*"},{"Effect":"Deny","Action":"s3:DeleteObject","Resource":"arn:aws:s3:::own/a.*"}]}"#,
    )
    .await;
    let document = r#"<Delete xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Object><Key>a.txt</Key></Object><Object><Key>b.txt</Key></Object></Delete>"#;
    let md5 = crate::delete_many::md5_of(document);
    let batch = call_as(
        &state,
        keeper.0.pair(),
        "POST",
        "/own",
        vec![("delete", None)],
        vec![
            ("content-md5", md5.as_str()),
            ("x-amz-security-token", &keeper.1),
        ],
        document.as_bytes(),
    )
    .await;
    assert_eq!(batch.status, 200, "{}", batch.body);
    assert!(
        batch.body.contains("<Deleted><Key>b.txt</Key>"),
        "{}",
        batch.body
    );
    assert!(
        batch
            .body
            .contains("<Error><Key>a.txt</Key><Code>AccessDenied</Code>"),
        "{}",
        batch.body
    );
    assert_eq!(
        as_session(&state, &keeper, "GET", "/own/a.txt").await,
        (200, None)
    );
    let unparsable = ask(
        &state,
        &asking(&provider.identity_token(Shape::Good), &[("Policy", r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:*","Resource":"*"}]}"#)], ""),
    )
    .await;
    assert_eq!(
        (unparsable.status, unparsable.code.as_deref()),
        (400, Some("MalformedPolicyDocument"))
    );
}
