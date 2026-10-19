//! What a client sees when a request is refused before any operation runs.

use crate::signer::{Unsigned, presign, sign};
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
async fn an_anonymous_request_is_denied_and_one_whose_bucket_cannot_be_read_is_an_outage() {
    let headers = [("Host".to_owned(), "localhost".to_owned())];
    // Nothing grants an anonymous caller the list of buckets, and no store is read to say so.
    let seen = send(state(NOW, 64), request("GET", "/", &headers)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (403, Some("AccessDenied"))
    );
    assert!(seen.request_id.is_some());
    // A bucket's policy decides an anonymous read; with no store to read it from that is not a denial but an outage.
    let unread = send(state(NOW, 64), request("GET", "/bucket/key", &headers)).await;
    assert_eq!(
        (unread.status, unread.code.as_deref()),
        (503, Some("ServiceUnavailable"))
    );
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
    assert_eq!(
        (seen.status, seen.body.as_str()),
        (503, ""),
        "HeadObject reaches the unreachable metadata store"
    );
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
    // No store answers this harness, so the anonymous read is refused as an outage; the envelope is the subject.
    assert_eq!(seen.code.as_deref(), Some("ServiceUnavailable"));
    assert!(
        seen.body
            .contains("<Resource>/bucket/a&amp;b&apos;c</Resource>"),
        "{}",
        seen.body
    );
}

#[tokio::test]
async fn a_version_2_signature_is_refused_as_an_unsupported_mechanism_in_either_form() {
    const MECHANISM: &str = "The authorization mechanism you have provided is not supported. Please use AWS4-HMAC-SHA256.";
    let header = [
        ("Host".to_owned(), "localhost".to_owned()),
        (
            "Authorization".to_owned(),
            format!("AWS {ACCESS_KEY}:c2lnbmF0dXJl"),
        ),
    ];
    let by_header = send(state(NOW, 64), request("GET", "/bucket/key", &header)).await;
    assert_eq!(
        (by_header.status, by_header.code.as_deref()),
        (400, Some("InvalidRequest")),
        "{}",
        by_header.body
    );
    assert!(by_header.body.contains(MECHANISM), "{}", by_header.body);

    let host = [("Host".to_owned(), "localhost".to_owned())];
    let target =
        format!("/bucket/key?AWSAccessKeyId={ACCESS_KEY}&Expires=1893456000&Signature=c2ln");
    let by_query = send(state(NOW, 64), request("GET", &target, &host)).await;
    assert_eq!(
        (by_query.status, by_query.code.as_deref()),
        (400, Some("InvalidRequest")),
        "{}",
        by_query.body
    );
    assert!(by_query.body.contains(MECHANISM), "{}", by_query.body);
}

fn presigned(method: &str, path: &str, query: Vec<(&str, Option<&str>)>) -> String {
    presign(
        &Unsigned {
            method,
            host: "localhost",
            path,
            query,
            headers: Vec::new(),
            access_key: ACCESS_KEY,
            secret: SECRET,
            region: "us-east-1",
            amz_date: AMZ_DATE,
        },
        300,
    )
}

#[tokio::test]
async fn a_presigned_url_carrying_what_sdks_add_is_dispatched_by_its_operation() {
    let host = [("Host".to_owned(), "localhost".to_owned())];
    // The payload marker, the operation name and a header the SDK moved into the signed query.
    let target = presigned(
        "GET",
        "/bucket/key",
        vec![
            ("X-Amz-Content-Sha256", Some("UNSIGNED-PAYLOAD")),
            ("x-amz-checksum-mode", Some("ENABLED")),
            ("x-id", Some("GetObject")),
        ],
    );
    let seen = send(state(NOW, 64), request("GET", &target, &host)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (503, Some("ServiceUnavailable")),
        "authenticated and dispatched to GetObject, which reaches the unreachable store: {}",
        seen.body
    );
}

#[tokio::test]
async fn a_presigned_url_declaring_a_payload_hash_or_contradicting_a_header_is_refused() {
    let host = [("Host".to_owned(), "localhost".to_owned())];
    let hashed = presigned(
        "GET",
        "/bucket/key",
        vec![(
            "X-Amz-Content-Sha256",
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        )],
    );
    let seen = send(state(NOW, 64), request("GET", &hashed, &host)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (400, Some("InvalidArgument")),
        "a presigned body is never hashed: {}",
        seen.body
    );
    assert!(seen.body.contains("UNSIGNED-PAYLOAD"), "{}", seen.body);
    let moved = presigned(
        "GET",
        "/bucket/key",
        vec![("x-amz-checksum-mode", Some("ENABLED"))],
    );
    let both = [
        ("Host".to_owned(), "localhost".to_owned()),
        ("x-amz-checksum-mode".to_owned(), "DISABLED".to_owned()),
    ];
    let seen = send(state(NOW, 64), request("GET", &moved, &both)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (400, Some("InvalidArgument")),
        "one header given twice with two values: {}",
        seen.body
    );
    assert!(
        seen.body
            .contains("x-amz-checksum-mode is given both as a header and in the query"),
        "{}",
        seen.body
    );
}
