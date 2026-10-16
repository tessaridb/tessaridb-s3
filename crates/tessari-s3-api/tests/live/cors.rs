//! CORS against a real node: a configuration stored and read back, a browser's preflight answered by the first
//! matching rule without credentials, every refusal alike whatever the reason, and an actual request — an error one
//! included — carrying the headers its rule allows.

use axum::body::Body;
use axum::http::Request;

use crate::{IGNORED, Seen, call, fresh, send};

const CONFIGURATION: &str = r#"<CORSConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
<CORSRule><ID>app</ID><AllowedOrigin>https://app.example.com</AllowedOrigin><AllowedMethod>PUT</AllowedMethod>
<AllowedMethod>GET</AllowedMethod><AllowedHeader>content-type</AllowedHeader><AllowedHeader>x-amz-*</AllowedHeader>
<ExposeHeader>ETag</ExposeHeader><MaxAgeSeconds>600</MaxAgeSeconds></CORSRule>
<CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule>
</CORSConfiguration>"#;

async fn preflight(
    state: &tessari_s3_api::ApiState,
    path: &str,
    origin: Option<&str>,
    method: Option<&str>,
    headers: Option<&str>,
) -> Seen {
    let mut request = Request::builder()
        .method("OPTIONS")
        .uri(path)
        .header("host", "localhost:9100");
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    if let Some(method) = method {
        request = request.header("access-control-request-method", method);
    }
    if let Some(headers) = headers {
        request = request.header("access-control-request-headers", headers);
    }
    send(state, request.body(Body::empty()).expect("request")).await
}

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_cors_configuration_is_stored_read_back_and_removed() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/site", vec![], b"").await.status, 200);
    let none = call(&state, "GET", "/site", vec![("cors", None)], b"").await;
    assert_eq!(
        (none.status, none.code.as_deref()),
        (404, Some("NoSuchCORSConfiguration"))
    );
    let put = call(
        &state,
        "PUT",
        "/site",
        vec![("cors", None)],
        CONFIGURATION.as_bytes(),
    )
    .await;
    assert_eq!(put.status, 200, "{}", put.body);
    let read = call(&state, "GET", "/site", vec![("cors", None)], b"").await;
    assert_eq!(read.status, 200, "{}", read.body);
    for part in [
        "<ID>app</ID>",
        "<AllowedOrigin>https://app.example.com</AllowedOrigin>",
        "<AllowedHeader>x-amz-*</AllowedHeader>",
        "<ExposeHeader>ETag</ExposeHeader>",
        "<MaxAgeSeconds>600</MaxAgeSeconds>",
        "<AllowedOrigin>*</AllowedOrigin>",
    ] {
        assert!(read.body.contains(part), "{part}: {}", read.body);
    }
    let two_stars = "<CORSConfiguration><CORSRule><AllowedOrigin>https://*.*.example.com</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>";
    let refused = call(
        &state,
        "PUT",
        "/site",
        vec![("cors", None)],
        two_stars.as_bytes(),
    )
    .await;
    assert_eq!(refused.code.as_deref(), Some("InvalidRequest"));
    let patch = "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>PATCH</AllowedMethod></CORSRule></CORSConfiguration>";
    assert_eq!(
        call(
            &state,
            "PUT",
            "/site",
            vec![("cors", None)],
            patch.as_bytes()
        )
        .await
        .code
        .as_deref(),
        Some("InvalidRequest")
    );
    let removed = call(&state, "DELETE", "/site", vec![("cors", None)], b"").await;
    assert_eq!(removed.status, 204, "{}", removed.body);
    assert_eq!(
        call(&state, "GET", "/site", vec![("cors", None)], b"")
            .await
            .code
            .as_deref(),
        Some("NoSuchCORSConfiguration")
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_preflight_is_answered_by_the_first_matching_rule_and_refused_alike_otherwise() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/site", vec![], b"").await.status, 200);
    assert_eq!(call(&state, "PUT", "/bare", vec![], b"").await.status, 200);
    let unset = preflight(&state, "/bare/a", Some("https://a.org"), Some("GET"), None).await;
    assert_eq!(
        (unset.status, unset.code.as_deref()),
        (403, Some("AccessForbidden"))
    );
    assert_eq!(
        call(
            &state,
            "PUT",
            "/site",
            vec![("cors", None)],
            CONFIGURATION.as_bytes()
        )
        .await
        .status,
        200
    );
    let allowed = preflight(
        &state,
        "/site/a.txt",
        Some("https://app.example.com"),
        Some("PUT"),
        Some("Content-Type, x-amz-date"),
    )
    .await;
    assert_eq!(allowed.status, 200, "{}", allowed.body);
    assert_eq!(
        header(&allowed, "access-control-allow-origin"),
        Some("https://app.example.com")
    );
    assert_eq!(
        header(&allowed, "access-control-allow-credentials"),
        Some("true")
    );
    assert_eq!(
        header(&allowed, "access-control-allow-methods"),
        Some("PUT, GET")
    );
    assert_eq!(
        header(&allowed, "access-control-allow-headers"),
        Some("content-type, x-amz-date")
    );
    assert_eq!(header(&allowed, "access-control-max-age"), Some("600"));
    assert_eq!(
        header(&allowed, "access-control-expose-headers"),
        Some("ETag")
    );
    assert!(header(&allowed, "vary").is_some_and(|vary| vary.contains("Origin")));
    let anyone = preflight(&state, "/site", Some("https://a.org"), Some("GET"), None).await;
    assert_eq!(anyone.status, 200, "{}", anyone.body);
    assert_eq!(header(&anyone, "access-control-allow-origin"), Some("*"));
    assert_eq!(header(&anyone, "access-control-allow-credentials"), None);
    // No rule, no bucket: the same refusal, so a preflight learns nothing about what exists.
    for path in ["/site/a.txt", "/no-such-bucket/a.txt"] {
        let refused = preflight(&state, path, Some("https://a.org"), Some("PUT"), None).await;
        assert_eq!(
            (refused.status, refused.code.as_deref()),
            (403, Some("AccessForbidden")),
            "{path}"
        );
        assert_eq!(header(&refused, "access-control-allow-origin"), None);
    }
    let headless = preflight(&state, "/site/a.txt", Some("https://a.org"), None, None).await;
    assert_eq!(
        (headless.status, headless.code.as_deref()),
        (400, Some("BadRequest"))
    );
    let originless = preflight(&state, "/site/a.txt", None, Some("GET"), None).await;
    assert_eq!(originless.code.as_deref(), Some("BadRequest"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_actual_request_carries_its_rules_headers_errors_included() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/site", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/site/a.txt", vec![], b"hello")
            .await
            .status,
        200
    );
    assert_eq!(
        call(
            &state,
            "PUT",
            "/site",
            vec![("cors", None)],
            CONFIGURATION.as_bytes()
        )
        .await
        .status,
        200
    );
    let origin = vec![("origin", "https://app.example.com")];
    let read = crate::call_with(&state, "GET", "/site/a.txt", vec![], origin.clone(), b"").await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert_eq!(
        header(&read, "access-control-allow-origin"),
        Some("https://app.example.com")
    );
    assert_eq!(header(&read, "access-control-expose-headers"), Some("ETag"));
    let missing = crate::call_with(&state, "GET", "/site/none.txt", vec![], origin, b"").await;
    assert_eq!(missing.status, 404, "{}", missing.body);
    assert_eq!(
        header(&missing, "access-control-allow-origin"),
        Some("https://app.example.com"),
        "a browser must be able to read the error"
    );
    let delete = crate::call_with(
        &state,
        "DELETE",
        "/site/a.txt",
        vec![],
        vec![("origin", "https://app.example.com")],
        b"",
    )
    .await;
    assert_eq!(delete.status, 204);
    assert_eq!(
        header(&delete, "access-control-allow-origin"),
        None,
        "no rule allows DELETE"
    );
}
