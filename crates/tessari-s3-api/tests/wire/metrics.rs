//! The metrics listener: what the S3 surface served, refused and shed is counted by operation, status and error code
//! and read back as Prometheus text from the node's own metrics router.

use axum::body::{Body, to_bytes};
use axum::http::Request;
use tessari_s3_api::{ApiState, metrics_router};
use tower::ServiceExt;

use crate::signer::{Unsigned, sign};
use crate::{ACCESS_KEY, AMZ_DATE, NOW, SECRET, request, send, state};

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

fn signed(access_key: &str) -> (String, Vec<(String, String)>) {
    sign(&Unsigned {
        method: "GET",
        host: "localhost:9100",
        path: "/bucket/key",
        query: Vec::new(),
        headers: Vec::new(),
        access_key,
        secret: SECRET,
        region: "us-east-1",
        amz_date: AMZ_DATE,
    })
}

/// The published presigned URL, valid for a day from `AMZ_DATE`.
const PRESIGNED: &str = "/test.txt?X-Amz-Algorithm=AWS4-HMAC-SHA256\
    &X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request\
    &X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host\
    &X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404";

#[tokio::test]
async fn authentication_refusals_are_counted_by_reason() {
    let at_now = state(NOW, 64);
    let (target, headers) = signed(ACCESS_KEY);
    let tampered = send(
        at_now.clone(),
        request("GET", &target.replace("key", "kez"), &headers),
    )
    .await;
    assert_eq!(tampered.code.as_deref(), Some("SignatureDoesNotMatch"));
    let (target, headers) = signed("AKIDUNKNOWN");
    let unknown = send(at_now.clone(), request("GET", &target, &headers)).await;
    assert_eq!(unknown.code.as_deref(), Some("InvalidAccessKeyId"));
    let anonymous = send(at_now.clone(), request("GET", "/bucket/key", &host())).await;
    assert_eq!(anonymous.code.as_deref(), Some("AccessDenied"));
    let text = scrape(at_now).await;
    for line in [
        "s3_auth_refusals_total{reason=\"signature\"} 1\n",
        "s3_auth_refusals_total{reason=\"unknown_key\"} 1\n",
        "s3_auth_refusals_total{reason=\"anonymous\"} 1\n",
    ] {
        assert!(text.contains(line), "missing {line}in\n{text}");
    }
    let late = state(NOW + 901, 64);
    let (target, headers) = signed(ACCESS_KEY);
    let skewed = send(late.clone(), request("GET", &target, &headers)).await;
    assert_eq!(skewed.code.as_deref(), Some("RequestTimeTooSkewed"));
    let presign_host = [(
        "Host".to_owned(),
        "examplebucket.s3.amazonaws.com".to_owned(),
    )];
    let after_a_day = state(NOW + 86_401, 64);
    let expired = send(
        after_a_day.clone(),
        request("GET", PRESIGNED, &presign_host),
    )
    .await;
    assert_eq!(
        expired.code.as_deref(),
        Some("AccessDenied"),
        "{}",
        expired.body
    );
    assert!(
        scrape(late)
            .await
            .contains("s3_auth_refusals_total{reason=\"skew\"} 1\n")
    );
    assert!(
        scrape(after_a_day)
            .await
            .contains("s3_auth_refusals_total{reason=\"expired\"} 1\n")
    );
}

#[tokio::test]
async fn a_backlog_the_metadata_store_cannot_answer_is_reported_down() {
    // The wire state's metadata store refuses every connection.
    let text = scrape(state(NOW, 64)).await;
    assert!(
        text.contains("s3_collect_up{source=\"replication\"} 0\n"),
        "{text}"
    );
    assert!(
        text.contains("s3_collect_up{source=\"cluster\"} 0\n"),
        "{text}"
    );
    assert!(!text.contains("s3_replication_backlog{"), "{text}");
}

#[tokio::test]
async fn a_full_access_log_queue_drops_the_record_and_still_serves_the_request() {
    let served = state(NOW, 64);
    // Started and never run: the one place in its queue stays taken.
    let _writer = served.audit_writer(1).expect("the access log starts once");
    assert!(served.audit_writer(1).is_none(), "and only once");
    for _ in 0..2 {
        let seen = send(served.clone(), request("GET", "/bucket/key", &host())).await;
        assert_eq!(seen.code.as_deref(), Some("AccessDenied"));
    }
    let text = scrape(served).await;
    assert!(text.contains("s3_audit_dropped_total 1\n"), "{text}");
    assert!(text.contains("s3_audit_written_total 0\n"), "{text}");
}
