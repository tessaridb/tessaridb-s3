//! Privileged acts — issuing a key or an upload key, linking an identity, setting or removing a grant, setting a
//! quota, deleting a bucket, staging, discarding or applying a layout — require that the operator authenticated within the last five minutes; an older session
//! is refused with `reauthentication_required` while ordinary reads go on. A key holder authenticates by signing in;
//! a person through the provider, whose `auth_time` is what counts, and a fresh start asks the provider for `max_age`.

use std::sync::atomic::Ordering;

use crate::IGNORED;
use crate::console::{Seen, send, sign_in};
use crate::console_oidc::console_with;
use crate::console_users::states;
use crate::oidc_provider::{Provider, SUBJECT, Shape};
use crate::users::user;

/// Every privileged act, each sent with a body its handler would otherwise read.
const PRIVILEGED: [(&str, &str, &str); 10] = [
    (
        "PUT",
        "/api/v1/cluster/layout/staged",
        r#"{"sets":[["n1"]],"reason":"r"}"#,
    ),
    (
        "DELETE",
        "/api/v1/cluster/layout/staged",
        r#"{"reason":"r"}"#,
    ),
    (
        "POST",
        "/api/v1/cluster/layout/apply",
        r#"{"id":"x","reason":"r"}"#,
    ),
    ("POST", "/api/v1/users/ann/keys", r#"{"reason":"r"}"#),
    (
        "POST",
        "/api/v1/buckets/fresh-b/upload-keys",
        r#"{"key":"k","expires_in":60,"reason":"r"}"#,
    ),
    (
        "PUT",
        "/api/v1/users/ann/identity",
        r#"{"subject":"s","reason":"r"}"#,
    ),
    (
        "PUT",
        "/api/v1/users/ann/grants/fresh-b",
        r#"{"read":true,"write":false,"reason":"r"}"#,
    ),
    (
        "DELETE",
        "/api/v1/users/ann/grants/fresh-b",
        r#"{"reason":"r"}"#,
    ),
    (
        "PUT",
        "/api/v1/buckets/fresh-b/quota",
        r#"{"max_bytes":1,"reason":"r"}"#,
    ),
    ("DELETE", "/api/v1/buckets/fresh-b", r#"{"reason":"r"}"#),
];

fn reauthentication(seen: &Seen) -> bool {
    seen.status == 401 && seen.code().as_deref() == Some("reauthentication_required")
}

async fn each_privileged(console: &tessari_s3_api::ConsoleState, token: &str) -> Vec<bool> {
    let mut answers = Vec::new();
    for (method, path, body) in PRIVILEGED {
        answers.push(reauthentication(
            &send(console, method, path, Some(token), Some(body)).await,
        ));
    }
    answers
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_key_session_does_privileged_acts_only_within_five_minutes_of_signing_in() {
    let _why = IGNORED;
    let (_, console, now) = states().await;
    let token = sign_in(&console).await;
    let signed_at = now.load(Ordering::Acquire);
    now.store(signed_at + 10, Ordering::Release);
    assert_eq!(
        each_privileged(&console, &token).await,
        [false; 10],
        "fresh: every act reaches its own checks"
    );
    now.store(signed_at + 301, Ordering::Release);
    assert_eq!(
        each_privileged(&console, &token).await,
        [true; 10],
        "stale: every act is refused for re-authentication"
    );
    // The session itself is still good for everything else.
    assert_eq!(
        send(&console, "GET", "/api/v1/buckets", Some(&token), None)
            .await
            .status,
        200
    );
    // Signing in again is the re-authentication.
    let again = sign_in(&console).await;
    assert_eq!(each_privileged(&console, &again).await, [false; 10]);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_persons_freshness_is_the_providers_auth_time_and_a_fresh_start_asks_for_max_age() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (console, storage) = console_with(&provider, None).await;
    storage
        .identities()
        .link(&user("ann"), &provider.issuer, SUBJECT)
        .await
        .expect("linked");
    let person = |code: &'static str, shape: Shape| {
        let console = console.clone();
        let provider = &provider;
        async move {
            let started = send(&console, "GET", "/api/v1/auth/oidc/start", None, None).await;
            let url = started.header("location").expect("a location").to_owned();
            let state = openidconnect::url::Url::parse(&url)
                .expect("a URL")
                .query_pairs()
                .find(|(key, _)| key == "state")
                .map(|(_, value)| value.into_owned())
                .expect("a state");
            provider.grant(&url, code, shape);
            send(
                &console,
                "GET",
                &format!("/api/v1/auth/oidc/callback?code={code}&state={state}"),
                None,
                None,
            )
            .await
            .cookie()
            .expect("a session")
        }
    };
    // Authenticated at the provider an hour ago (its session reused): signed in, but not for privileged acts.
    let stale = person("code-stale", Shape::StaleAuth).await;
    assert_eq!(each_privileged(&console, &stale).await, [true; 10]);
    // Saying nothing about when counts as long ago.
    let unknown = person("code-unknown", Shape::NoAuthTime).await;
    assert_eq!(each_privileged(&console, &unknown).await, [true; 10]);
    // Authenticated seconds ago: the acts reach their own checks.
    let fresh = person("code-fresh", Shape::Good).await;
    assert_eq!(each_privileged(&console, &fresh).await, [false; 10]);
    // The way to a fresh session: the start asks the provider for a recent authentication.
    let started = send(
        &console,
        "GET",
        "/api/v1/auth/oidc/start?fresh=true",
        None,
        None,
    )
    .await;
    assert_eq!(started.status, 303);
    let location = started.header("location").expect("a location");
    let max_age = openidconnect::url::Url::parse(location)
        .expect("a URL")
        .query_pairs()
        .find(|(key, _)| key == "max_age")
        .map(|(_, value)| value.into_owned());
    assert_eq!(max_age.as_deref(), Some("300"));
}
