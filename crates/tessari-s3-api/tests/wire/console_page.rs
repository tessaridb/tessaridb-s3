//! The console's page and its assets: served without a session because they carry no data, each under a policy that
//! lets the page load only its own script and style and never be framed, and limited per client address like any
//! other public route.

use std::net::SocketAddr;

use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request};
use tessari_s3_api::{ConsoleState, console_router};
use tower::ServiceExt;

use super::console::console;
use crate::NOW;

struct Fetched {
    status: u16,
    headers: HeaderMap,
    body: String,
}

async fn get(state: &ConsoleState, path: &str, from: [u8; 4]) -> Fetched {
    let mut request = Request::builder()
        .uri(path)
        .body(Body::empty())
        .expect("request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((from, 40_000))));
    let response = console_router(state.clone())
        .oneshot(request)
        .await
        .expect("infallible");
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1 << 22).await.expect("body");
    Fetched {
        status,
        headers,
        body: String::from_utf8(bytes.to_vec()).expect("utf-8"),
    }
}

fn header<'a>(fetched: &'a Fetched, name: &str) -> &'a str {
    fetched
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

/// Every asset the page is made of, with the type it must be served as.
const ASSETS: [(&str, &str); 4] = [
    ("/", "text/html; charset=utf-8"),
    ("/console.js", "text/javascript; charset=utf-8"),
    ("/console.css", "text/css; charset=utf-8"),
    ("/favicon.svg", "image/svg+xml"),
];

#[tokio::test]
async fn the_page_and_its_assets_are_served_without_a_session_under_a_strict_policy() {
    let state = console(NOW, "10", "600");
    for (path, kind) in ASSETS {
        let fetched = get(&state, path, [10, 0, 0, 1]).await;
        assert_eq!(fetched.status, 200, "{path}: {}", fetched.body);
        assert_eq!(header(&fetched, "content-type"), kind, "{path}");
        assert!(!fetched.body.is_empty(), "{path} is empty");
        let policy = header(&fetched, "content-security-policy");
        for directive in [
            "default-src 'none'",
            "script-src 'self'",
            "style-src 'self'",
            "connect-src 'self'",
            "frame-ancestors 'none'",
            "base-uri 'none'",
            "form-action 'none'",
        ] {
            assert!(
                policy.contains(directive),
                "{path}: {directive} in {policy:?}"
            );
        }
        assert_eq!(
            header(&fetched, "x-content-type-options"),
            "nosniff",
            "{path}"
        );
        assert_eq!(header(&fetched, "referrer-policy"), "no-referrer", "{path}");
    }
    let page = get(&state, "/", [10, 0, 0, 1]).await;
    for linked in ["/console.js", "/console.css", "/favicon.svg"] {
        assert!(page.body.contains(linked), "the page links {linked}");
    }
}

#[tokio::test]
async fn the_page_is_limited_per_client_address() {
    let state = console(NOW, "10", "2");
    for _ in 0..2 {
        assert_eq!(get(&state, "/", [10, 0, 0, 7]).await.status, 200);
    }
    let refused = get(&state, "/console.js", [10, 0, 0, 7]).await;
    assert_eq!(refused.status, 429, "{}", refused.body);
    assert!(
        refused.body.contains("\"code\":\"rate_limit\""),
        "{}",
        refused.body
    );
    assert_eq!(get(&state, "/", [10, 0, 0, 8]).await.status, 200);
}
