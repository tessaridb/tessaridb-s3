//! A person's console session ends when the identity provider says so: a back-channel logout token the console
//! believes ends every session of the user that identity is linked to, at its next request, and so does unlinking
//! the identity. A token that fails a check, or arrives a second time, is refused and ends nothing; a key's session
//! is not a person's and is untouched. The time from the logout's answer to the refusal is measured.

use std::net::SocketAddr;
use std::time::Instant;

use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::Request;
use tessari_s3_api::{ConsoleState, console_router};
use tower::ServiceExt;

use crate::IGNORED;
use crate::console::{send, sign_in};
use crate::console_oidc::console_with;
use crate::oidc_provider::{Logout, Provider, SUBJECT, Shape};
use crate::users::user;

/// Posts `logout_token` as the provider would; the status and the `cache-control` header.
async fn deliver(console: &ConsoleState, token: &str) -> (u16, Option<String>) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/oidc/backchannel-logout")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("logout_token={token}")))
        .expect("request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([10, 0, 0, 7], 40_001))));
    let response = console_router(console.clone())
        .oneshot(request)
        .await
        .expect("infallible");
    let status = response.status().as_u16();
    let cache = response
        .headers()
        .get("cache-control")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let _ = to_bytes(response.into_body(), 1 << 16).await;
    (status, cache)
}

/// A person signed in through the provider: the session token.
async fn person(console: &ConsoleState, provider: &Provider, code: &str) -> String {
    let started = send(console, "GET", "/api/v1/auth/oidc/start", None, None).await;
    let url = started.header("location").expect("a location").to_owned();
    let state = openidconnect::url::Url::parse(&url)
        .expect("a URL")
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .expect("a state");
    provider.grant(&url, code, Shape::Good);
    send(
        console,
        "GET",
        &format!("/api/v1/auth/oidc/callback?code={code}&state={state}"),
        None,
        None,
    )
    .await
    .cookie()
    .expect("a session")
}

async fn status_with(console: &ConsoleState, token: &str) -> u16 {
    send(console, "GET", "/api/v1/session", Some(token), None)
        .await
        .status
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_providers_logout_ends_the_persons_sessions_at_their_next_request_and_only_once() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (console, storage) = console_with(&provider, None).await;
    storage
        .identities()
        .link(&user("ann"), &provider.issuer, SUBJECT)
        .await
        .expect("linked");
    let ann = person(&console, &provider, "code-ann").await;
    let root = sign_in(&console).await;
    assert_eq!(status_with(&console, &ann).await, 200);

    // Tokens that fail a check end nothing.
    for (round, how) in [Logout::WrongAudience, Logout::Tampered, Logout::WithNonce]
        .into_iter()
        .enumerate()
    {
        let token = provider.logout_token(SUBJECT, &format!("jti-bad-{round}"), how);
        assert_eq!(deliver(&console, &token).await.0, 400, "{how:?}");
    }
    assert_eq!(status_with(&console, &ann).await, 200, "still signed in");

    // A subject linked to nobody is nothing to end.
    let stranger = provider.logout_token("nobody-here", "jti-stranger", Logout::Good);
    assert_eq!(deliver(&console, &stranger).await.0, 200);

    let token = provider.logout_token(SUBJECT, "jti-ann", Logout::Good);
    let delivered = deliver(&console, &token).await;
    let answered = Instant::now();
    assert_eq!(delivered, (200, Some("no-store".to_owned())));
    assert_eq!(
        status_with(&console, &ann).await,
        401,
        "ended at its next request"
    );
    let measured = answered.elapsed();
    eprintln!("deprovisioning: logout answered -> session refused in {measured:?}");
    // The same token again is refused: acted on once.
    assert_eq!(deliver(&console, &token).await.0, 400);
    // A key's session is not a person's.
    assert_eq!(status_with(&console, &root).await, 200);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn unlinking_an_identity_ends_the_sessions_it_opened() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (console, _) = console_with(&provider, None).await;
    let root = sign_in(&console).await;
    let linked = send(
        &console,
        "PUT",
        "/api/v1/users/ann/identity",
        Some(&root),
        Some(&format!(
            "{{\"subject\":\"{SUBJECT}\",\"reason\":\"ticket 9\"}}"
        )),
    )
    .await;
    assert_eq!(linked.status, 204);
    let ann = person(&console, &provider, "code-ann-2").await;
    assert_eq!(status_with(&console, &ann).await, 200);
    let unlinked = send(
        &console,
        "DELETE",
        "/api/v1/users/ann/identity",
        Some(&root),
        Some("{\"reason\":\"ticket 10\"}"),
    )
    .await;
    assert_eq!(unlinked.status, 204);
    assert_eq!(status_with(&console, &ann).await, 401);
}
