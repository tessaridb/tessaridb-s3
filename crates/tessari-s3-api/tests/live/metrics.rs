//! What a node served is in its metrics: a write, a read and a read of a missing key, each counted by operation,
//! status and code, and timed in the size class of the bytes it moved.

use axum::body::{Body, to_bytes};
use axum::http::Request;
use tessari_s3_api::{ApiState, metrics_router};
use tower::ServiceExt;

use crate::{IGNORED, call, fresh};

async fn scrape(state: ApiState) -> String {
    let response = metrics_router(state)
        .oneshot(
            Request::get("/metrics")
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("the router is infallible");
    String::from_utf8(
        to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("utf-8")
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn served_requests_are_counted_by_operation_status_code_and_size() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/counted", vec![], b"").await.status,
        200
    );
    let body = vec![7_u8; 100 << 10];
    let put = call(&state, "PUT", "/counted/one", vec![], &body).await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(
        call(&state, "GET", "/counted/one", vec![], b"")
            .await
            .status,
        200
    );
    let missing = call(&state, "GET", "/counted/none", vec![], b"").await;
    assert_eq!(missing.status, 404);
    let text = scrape(state).await;
    for line in [
        "s3_requests_total{operation=\"CreateBucket\",status=\"200\",code=\"None\"} 1\n",
        "s3_requests_total{operation=\"PutObject\",status=\"200\",code=\"None\"} 1\n",
        "s3_requests_total{operation=\"GetObject\",status=\"200\",code=\"None\"} 1\n",
        "s3_requests_total{operation=\"GetObject\",status=\"404\",code=\"NoSuchKey\"} 1\n",
        // The write's body and the read's answer are 100 KiB; the miss moved only its error document.
        "s3_request_duration_seconds_count{operation=\"PutObject\",size=\"lt_1MiB\"} 1\n",
        "s3_request_duration_seconds_count{operation=\"GetObject\",size=\"lt_1MiB\"} 1\n",
        "s3_request_duration_seconds_count{operation=\"GetObject\",size=\"lt_64KiB\"} 1\n",
    ] {
        assert!(text.contains(line), "missing {line}in\n{text}");
    }
}
