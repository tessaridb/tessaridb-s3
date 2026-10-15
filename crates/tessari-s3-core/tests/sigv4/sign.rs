//! The client-side signer: it reproduces a published signature, and what it signs the verifier accepts.

use tessari_s3_core::auth::{AuthError, SignedRequest, amz_date, sign_header};

use crate::{EMPTY_SHA256, NOW, REGION, check, secret};

const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

fn signed(
    method: &str,
    path: &str,
    query: &str,
    headers: &[(&str, String)],
) -> Vec<(&'static str, String)> {
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let request = SignedRequest {
        method,
        raw_path: path,
        raw_query: query,
        headers: &borrowed,
    };
    let authorization = sign_header(&request, ACCESS_KEY, &secret(), REGION).expect("signs");
    let mut owned: Vec<(&'static str, String)> =
        headers.iter().map(|(n, v)| (leak(n), v.clone())).collect();
    owned.push(("Authorization", authorization));
    owned
}

fn leak(name: &str) -> &'static str {
    Box::leak(name.to_owned().into_boxed_str())
}

#[test]
fn the_published_get_example_signs_to_the_published_signature() {
    let headers = [
        ("Host", "examplebucket.s3.amazonaws.com".to_owned()),
        ("Range", "bytes=0-9".to_owned()),
        ("x-amz-content-sha256", EMPTY_SHA256.to_owned()),
        ("x-amz-date", "20130524T000000Z".to_owned()),
    ];
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let request = SignedRequest {
        method: "GET",
        raw_path: "/test.txt",
        raw_query: "",
        headers: &borrowed,
    };
    assert_eq!(
        sign_header(&request, ACCESS_KEY, &secret(), REGION).as_deref(),
        Ok(
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
            SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,\
            Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        )
    );
}

#[test]
fn what_is_signed_with_a_query_and_mixed_case_headers_verifies() {
    let headers = [
        ("Host", "localhost:47920".to_owned()),
        ("X-Amz-Date", "20130524T000000Z".to_owned()),
        ("X-Amz-Content-Sha256", EMPTY_SHA256.to_owned()),
        ("Content-Type", "application/octet-stream".to_owned()),
    ];
    let owned = signed(
        "GET",
        "/bench/a%20b",
        "list-type=2&prefix=p%2F1&max-keys=1000",
        &headers,
    );
    // AWS requires the list lowercase and sorted; the verifier takes the order it is given, so it is pinned here.
    let authorization = &owned.last().expect("authorization").1;
    assert!(
        authorization.contains("SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date,"),
        "{authorization}"
    );
    let verified = check(
        "GET",
        "/bench/a%20b",
        "list-type=2&prefix=p%2F1&max-keys=1000",
        &owned,
        REGION,
        NOW,
    )
    .expect("the verifier accepts the signer");
    assert_eq!(verified.access_key(), ACCESS_KEY);
    // Control arm: the same signature over a changed query is refused, so the acceptance above is about this request.
    assert_eq!(
        check(
            "GET",
            "/bench/a%20b",
            "list-type=2&prefix=p%2F2&max-keys=1000",
            &owned,
            REGION,
            NOW
        )
        .map(|_| ()),
        Err(AuthError::SignatureMismatch)
    );
}

#[test]
fn the_date_header_text_is_the_compact_utc_form() {
    assert_eq!(amz_date(NOW), "20130524T000000Z");
    assert_eq!(amz_date(1_760_491_037), "20251015T011717Z");
}
