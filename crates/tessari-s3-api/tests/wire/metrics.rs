//! The metrics listener: what the S3 surface served, refused and shed is counted by operation, status and error code
//! and read back as Prometheus text from the node's own metrics router.

use axum::body::{Body, to_bytes};
use axum::http::Request;
use tessari_s3_api::{ApiState, metrics_router};
use tower::ServiceExt;

use crate::{NOW, request, send, state};

/// The metrics text `state` exposes.
async fn scrape(state: ApiState) -> String {
    let response = metrics_router(state)
        .oneshot(
            Request::get("/metrics")
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("the router is infallible");
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );
    String::from_utf8(
        to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("utf-8")
}

fn host() -> [(String, String); 1] {
    [("Host".to_owned(), "localhost".to_owned())]
}

#[tokio::test]
async fn a_refused_request_is_counted_by_its_operation_status_and_code() {
    let served = state(NOW, 64);
    let seen = send(served.clone(), request("GET", "/bucket/key", &host())).await;
    assert_eq!(seen.status, 403);
    let text = scrape(served).await;
    assert!(
        text.contains(
            "s3_requests_total{operation=\"GetObject\",status=\"403\",code=\"AccessDenied\"} 1\n"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "s3_request_duration_seconds_count{operation=\"GetObject\",size=\"lt_64KiB\"} 1\n"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn a_request_refused_before_dispatch_is_counted_as_unknown() {
    let served = state(NOW, 64);
    let seen = send(served.clone(), request("GET", "/Bad_Bucket/key", &host())).await;
    assert_eq!(seen.code.as_deref(), Some("InvalidBucketName"));
    let text = scrape(served).await;
    assert!(
        text.contains(
            "s3_requests_total{operation=\"Unknown\",status=\"400\",code=\"InvalidBucketName\"} 1\n"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn a_shed_request_is_counted_and_nothing_else_is() {
    let served = state(NOW, 0);
    let seen = send(served.clone(), request("GET", "/bucket/key", &host())).await;
    assert_eq!(seen.code.as_deref(), Some("SlowDown"));
    let text = scrape(served).await;
    assert!(text.contains("s3_requests_shed_total 1\n"), "{text}");
    assert!(!text.contains("s3_requests_total{"), "{text}");
}
