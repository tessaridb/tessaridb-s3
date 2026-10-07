//! What a client sees when a request is refused before any operation runs.

use crate::signer::{Unsigned, sign};
use crate::{ACCESS_KEY, AMZ_DATE, NOW, SECRET, request, send, state};

fn signed(
    method: &str,
    path: &str,
    query: Vec<(&str, Option<&str>)>,
    access_key: &str,
) -> (String, Vec<(String, String)>) {
    sign(&Unsigned {
        method,
        host: "localhost:9100",
        path,
        query,
        headers: Vec::new(),
        access_key,
        secret: SECRET,
        region: "us-east-1",
        amz_date: AMZ_DATE,
    })
}

#[tokio::test]
async fn an_anonymous_request_is_access_denied() {
    let headers = [("Host".to_owned(), "localhost".to_owned())];
    let seen = send(state(NOW, 64), request("GET", "/bucket/key", &headers)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (403, Some("AccessDenied"))
    );
    assert!(seen.request_id.is_some());
}

#[tokio::test]
async fn an_unknown_access_key_is_invalid_access_key_id() {
    let (target, headers) = signed("GET", "/bucket/key", Vec::new(), "AKIDUNKNOWN");
    let seen = send(state(NOW, 64), request("GET", &target, &headers)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (403, Some("InvalidAccessKeyId"))
    );
}

#[tokio::test]
async fn a_tampered_signed_request_is_signature_does_not_match() {
    let (target, headers) = signed("GET", "/bucket/key", Vec::new(), ACCESS_KEY);
    let seen = send(
        state(NOW, 64),
        request("GET", &target.replace("key", "kez"), &headers),
    )
    .await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (403, Some("SignatureDoesNotMatch"))
    );
}

#[tokio::test]
async fn a_bucket_subresource_on_an_object_is_method_not_allowed_never_put_object() {
    let (target, headers) = signed("PUT", "/bucket/key", vec![("cors", None)], ACCESS_KEY);
    let seen = send(state(NOW, 64), request("PUT", &target, &headers)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (405, Some("MethodNotAllowed")),
        "{}",
        seen.body
    );
}

#[tokio::test]
async fn an_invalid_bucket_name_is_refused_before_authentication() {
    let headers = [("Host".to_owned(), "localhost".to_owned())];
    let seen = send(state(NOW, 64), request("GET", "/Bad_Bucket/key", &headers)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (400, Some("InvalidBucketName"))
    );
}

#[tokio::test]
async fn a_head_refusal_carries_its_status_and_no_body() {
    let (target, headers) = signed("HEAD", "/bucket/key", Vec::new(), ACCESS_KEY);
    let seen = send(state(NOW, 64), request("HEAD", &target, &headers)).await;
    assert_eq!((seen.status, seen.body.as_str()), (501, ""));
    assert!(seen.request_id.is_some(), "the request id is on HEAD too");
}

#[tokio::test]
async fn past_the_in_flight_limit_a_request_is_slow_down_before_any_work() {
    let (target, headers) = signed("GET", "/bucket/key", Vec::new(), ACCESS_KEY);
    let seen = send(state(NOW, 0), request("GET", &target, &headers)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (503, Some("SlowDown")),
        "{}",
        seen.body
    );
    assert!(seen.request_id.is_some());
}

#[tokio::test]
async fn markup_in_the_path_cannot_break_out_of_the_error_envelope() {
    let headers = [("Host".to_owned(), "localhost".to_owned())];
    let seen = send(state(NOW, 64), request("GET", "/bucket/a&b'c", &headers)).await;
    assert_eq!(seen.code.as_deref(), Some("AccessDenied"));
    assert!(
        seen.body
            .contains("<Resource>/bucket/a&amp;b&apos;c</Resource>"),
        "{}",
        seen.body
    );
}
