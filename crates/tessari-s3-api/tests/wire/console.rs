//! The console's API probed with no browser in the loop: signing in with the root credential issues a session cookie
//! that is HttpOnly and SameSite=Strict; every protected route refuses a request without a session, with a tampered
//! one and with an expired one; sign-ins are limited per client address and requests per signed-in key.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Request, Response};
use tessari_s3_api::{ConsoleState, console_router};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_storage::Storage;
use tower::ServiceExt;

use crate::{ACCESS_KEY, NOW, SECRET};

pub(super) fn console(now: i64, sign_ins: &str, requests: &str) -> ConsoleState {
    let vars = [
        ("TESSARIDB_S3_ROOT_ACCESS_KEY", ACCESS_KEY),
        ("TESSARIDB_S3_ROOT_SECRET_KEY", SECRET),
        ("TESSARIDB_S3_META_ADDRESS", "127.0.0.1:1"),
        ("TESSARIDB_S3_META_USER", "s3"),
        ("TESSARIDB_S3_META_PASSWORD", "unused-password"),
        ("TESSARIDB_S3_CONSOLE_LISTEN", "127.0.0.1:9101"),
        ("TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE", sign_ins),
        ("TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE", requests),
    ];
    let config = S3Config::from_lookup(|key| {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_owned())
    })
    .expect("configuration");
    let storage = Storage::new(MetaPool::new(config.meta.clone()).expect("pool"), None);
    ConsoleState::new(&config, Arc::new(move || now), storage).expect("a console is configured")
}

struct Answer {
    status: u16,
    cookie: Option<String>,
    code: Option<String>,
    body: String,
}

async fn send(
    state: &ConsoleState,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<(&str, &str)>,
    from: [u8; 4],
) -> Answer {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = cookie {
        request = request.header("cookie", format!("tessaridb_s3_console={token}"));
    }
    let request = match body {
        Some((content_type, text)) => request
            .header("content-type", content_type)
            .body(Body::from(text.to_owned())),
        None => request.body(Body::empty()),
    };
    let mut request = request.expect("request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((from, 40_000))));
    let response: Response<Body> = console_router(state.clone())
        .oneshot(request)
        .await
        .expect("infallible");
    let status = response.status().as_u16();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("utf-8");
    let code = body
        .split("\"code\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .map(str::to_owned);
    Answer {
        status,
        cookie,
        code,
        body,
    }
}

const HERE: [u8; 4] = [10, 0, 0, 1];

fn credentials(key: &str, secret: &str) -> String {
    format!("{{\"access_key_id\":\"{key}\",\"secret_access_key\":\"{secret}\"}}")
}

async fn sign_in(state: &ConsoleState) -> String {
    let answer = send(
        state,
        "POST",
        "/api/v1/session",
        None,
        Some(("application/json", &credentials(ACCESS_KEY, SECRET))),
        HERE,
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let cookie = answer.cookie.expect("a session cookie");
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(cookie.contains(attribute), "{cookie}");
    }
    cookie
        .trim_start_matches("tessaridb_s3_console=")
        .split(';')
        .next()
        .expect("a value")
        .to_owned()
}

/// Every route that needs a session, as (method, path).
const PROTECTED: [(&str, &str); 13] = [
    ("GET", "/api/v1/status"),
    ("DELETE", "/api/v1/session"),
    ("GET", "/api/v1/buckets"),
    ("POST", "/api/v1/buckets"),
    ("DELETE", "/api/v1/buckets/bkt"),
    ("GET", "/api/v1/buckets/bkt/objects"),
    ("GET", "/api/v1/buckets/bkt/object?key=k"),
    ("DELETE", "/api/v1/buckets/bkt/object?key=k"),
    ("GET", "/api/v1/buckets/bkt/object/content?key=k"),
    ("GET", "/api/v1/actions"),
    ("GET", "/api/v1/usage"),
    ("PUT", "/api/v1/buckets"),
    ("GET", "/api/v1/not-a-route"),
];

#[tokio::test]
async fn the_root_credential_signs_in_and_its_session_reaches_the_status() {
    let state = console(NOW, "10", "600");
    let token = sign_in(&state).await;
    let status = send(&state, "GET", "/api/v1/status", Some(&token), None, HERE).await;
    assert_eq!(status.status, 200, "{}", status.body);
    assert!(status.body.contains("\"version\""), "{}", status.body);
    let out = send(
        &state,
        "DELETE",
        "/api/v1/session",
        Some(&token),
        None,
        HERE,
    )
    .await;
    assert_eq!(out.status, 204);
    assert!(
        out.cookie
            .as_deref()
            .is_some_and(|c| c.contains("Max-Age=0")),
        "{:?}",
        out.cookie
    );
}

#[tokio::test]
async fn a_wrong_or_malformed_credential_is_refused_without_a_session() {
    let state = console(NOW, "10", "600");
    for (content_type, body, expected) in [
        (
            "application/json",
            credentials(ACCESS_KEY, "wrong-secret-0123456789"),
            401,
        ),
        ("application/json", credentials("AKIAOTHERKEY", SECRET), 401),
        ("application/json", "{\"access_key_id\":1}".to_owned(), 400),
        ("text/plain", credentials(ACCESS_KEY, SECRET), 415),
    ] {
        let answer = send(
            &state,
            "POST",
            "/api/v1/session",
            None,
            Some((content_type, &body)),
            HERE,
        )
        .await;
        assert_eq!(answer.status, expected, "{body}: {}", answer.body);
        assert!(answer.cookie.is_none(), "{body}");
        assert!(answer.code.is_some(), "a JSON error: {}", answer.body);
    }
}

#[tokio::test]
async fn every_protected_route_refuses_no_session_a_tampered_one_and_an_expired_one() {
    let state = console(NOW, "10", "600");
    let token = sign_in(&state).await;
    let mut tampered = token.clone();
    tampered.push('A');
    let later = console(NOW + 3_600, "10", "600");
    for (method, path) in PROTECTED {
        for (who, state, cookie) in [
            ("no session", &state, None),
            ("tampered", &state, Some(tampered.as_str())),
            ("expired", &later, Some(token.as_str())),
        ] {
            let answer = send(state, method, path, cookie, None, HERE).await;
            assert_eq!(
                (answer.status, answer.code.as_deref()),
                (401, Some("unauthorized")),
                "{method} {path} {who}: {}",
                answer.body
            );
        }
    }
}

#[tokio::test]
async fn sign_ins_are_limited_per_client_address() {
    let state = console(NOW, "3", "600");
    let wrong = credentials(ACCESS_KEY, "wrong-secret-0123456789");
    for _ in 0..3 {
        let answer = send(
            &state,
            "POST",
            "/api/v1/session",
            None,
            Some(("application/json", &wrong)),
            HERE,
        )
        .await;
        assert_eq!(answer.status, 401);
    }
    // The right secret too: the limit is checked before the credential.
    let right = credentials(ACCESS_KEY, SECRET);
    let limited = send(
        &state,
        "POST",
        "/api/v1/session",
        None,
        Some(("application/json", &right)),
        HERE,
    )
    .await;
    assert_eq!(
        (limited.status, limited.code.as_deref()),
        (429, Some("rate_limit"))
    );
    let elsewhere = send(
        &state,
        "POST",
        "/api/v1/session",
        None,
        Some(("application/json", &right)),
        [10, 0, 0, 2],
    )
    .await;
    assert_eq!(elsewhere.status, 200, "another address is not limited");
}

#[tokio::test]
async fn requests_are_limited_per_signed_in_key() {
    let state = console(NOW, "10", "2");
    let token = sign_in(&state).await;
    for _ in 0..2 {
        let answer = send(&state, "GET", "/api/v1/status", Some(&token), None, HERE).await;
        assert_eq!(answer.status, 200);
    }
    let limited = send(&state, "GET", "/api/v1/status", Some(&token), None, HERE).await;
    assert_eq!(
        (limited.status, limited.code.as_deref()),
        (429, Some("rate_limit"))
    );
}

#[tokio::test]
async fn a_change_without_its_reason_or_with_a_bad_body_is_refused_before_storage() {
    // The metadata address of this state answers nothing, so reaching storage would be a 503, not these answers.
    let state = console(NOW, "10", "600");
    let token = sign_in(&state).await;
    let long = format!("{{\"reason\":\"{}\"}}", "x".repeat(501));
    for (method, path, content_type, body, expected) in [
        (
            "DELETE",
            "/api/v1/buckets/bkt",
            "application/json",
            "{}",
            (400, "reason_required"),
        ),
        (
            "DELETE",
            "/api/v1/buckets/bkt",
            "application/json",
            "{\"reason\":\"   \"}",
            (400, "reason_required"),
        ),
        (
            "DELETE",
            "/api/v1/buckets/bkt",
            "application/json",
            long.as_str(),
            (400, "reason_too_long"),
        ),
        (
            "DELETE",
            "/api/v1/buckets/bkt",
            "text/plain",
            "{\"reason\":\"cleanup\"}",
            (415, "unsupported_media_type"),
        ),
        (
            "DELETE",
            "/api/v1/buckets/bkt/object?key=k",
            "application/json",
            "{\"reason\":\"cleanup\"}",
            (400, "bad_request"),
        ),
        (
            "DELETE",
            "/api/v1/buckets/bkt/object?key=k",
            "application/json",
            "{\"etag\":\"\\\"e\\\"\"}",
            (400, "reason_required"),
        ),
        (
            "POST",
            "/api/v1/buckets",
            "application/json",
            "{\"name\":\"Not_A_Bucket\"}",
            (400, "invalid_bucket_name"),
        ),
        (
            "GET",
            "/api/v1/actions?limit=many",
            "application/json",
            "",
            (400, "bad_request"),
        ),
    ] {
        let answer = send(
            &state,
            method,
            path,
            Some(&token),
            Some((content_type, body)),
            HERE,
        )
        .await;
        assert_eq!(
            (answer.status, answer.code.as_deref()),
            (expected.0, Some(expected.1)),
            "{method} {path} {body}: {}",
            answer.body
        );
    }
}
