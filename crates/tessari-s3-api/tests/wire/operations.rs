//! Every operation of the catalog, signed and sent, is answered `NotImplemented` (501) on the wire; AWS's own
//! published requests authenticate and reach the same answer.

use tessari_s3_core::dispatch::{
    CATALOG, DispatchRequest, Operation, Target, dispatch, is_implemented,
};

use crate::signer::{EMPTY_SHA256, Unsigned, sign};
use crate::{ACCESS_KEY, AMZ_DATE, NOW, SECRET, request, send, state};

#[tokio::test]
async fn every_catalog_operation_signed_and_sent_reaches_its_handler_or_is_not_implemented() {
    let mut failures = Vec::new();
    for spec in &CATALOG {
        let path = match spec.target {
            Target::Service => "/".to_owned(),
            Target::Bucket => "/bucket".to_owned(),
            Target::Object => "/bucket/key".to_owned(),
            Target::Named(path) => path.to_owned(),
        };
        let mut query: Vec<(&str, Option<&str>)> = spec.discriminators.to_vec();
        query.extend(spec.required_query.iter().map(|name| (*name, Some("1"))));
        let headers: Vec<(&str, &str)> = spec
            .required_headers
            .iter()
            .map(|name| (*name, "value"))
            .collect();
        let method = format!("{:?}", spec.method).to_ascii_uppercase();
        let unsigned = Unsigned {
            method: &method,
            host: "localhost:9100",
            path: &path,
            query,
            headers,
            access_key: ACCESS_KEY,
            secret: SECRET,
            region: "us-east-1",
            amz_date: AMZ_DATE,
        };
        let (target, signed) = sign(&unsigned);
        // Every client sends Content-Length (0 here): PutObject refuses a body of unknown length before anything else.
        let mut signed = signed;
        signed.push(("content-length".to_owned(), "0".to_owned()));
        let seen = send(state(NOW, 64), request(&method, &target, &signed)).await;
        // An implemented operation reaches its handler, which meets the unreachable metadata store and answers
        // 503 ServiceUnavailable — fail closed, never a guess.
        // A shadowed row is answered by the operation that owns its wire shape, so judge by that one.
        let dispatch_query: Vec<(&str, Option<&str>)> = spec
            .discriminators
            .iter()
            .copied()
            .chain(spec.required_query.iter().map(|name| (*name, Some("1"))))
            .collect();
        let answering = dispatch(&DispatchRequest {
            method: spec.method,
            target: spec.target,
            query: &dispatch_query,
            header_names: spec.required_headers,
        })
        .map_or(spec.operation, |answering| answering.operation);
        let (status, code) = if answering == Operation::DeleteObjects {
            // Its handler refuses a body with neither Content-MD5 nor a checksum before reading the store.
            (400, "InvalidRequest")
        } else if matches!(
            answering,
            Operation::UploadPart | Operation::AbortMultipartUpload | Operation::ListParts
        ) {
            // `uploadId=1` is not an id this server could have issued: refused before reading the store.
            (404, "NoSuchUpload")
        } else if is_implemented(answering) {
            (503, "ServiceUnavailable")
        } else {
            (501, "NotImplemented")
        };
        let head_ok = method == "HEAD" && seen.status == status && seen.body.is_empty();
        if !(head_ok || (seen.status == status && seen.code.as_deref() == Some(code))) {
            failures.push(format!(
                "{} {method} {target}: {} {:?}",
                spec.name, seen.status, seen.code
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of 116 operations:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn the_published_get_object_request_authenticates_and_reaches_its_handler() {
    let authorization = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
        SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,\
        Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41";
    let headers = [
        ("Host", "examplebucket.s3.amazonaws.com"),
        ("Authorization", authorization),
        ("Range", "bytes=0-9"),
        ("x-amz-content-sha256", EMPTY_SHA256),
        ("x-amz-date", AMZ_DATE),
    ];
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect();
    let seen = send(state(NOW, 64), request("GET", "/test.txt", &headers)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (503, Some("ServiceUnavailable")),
        "{}",
        seen.body
    );
    assert!(
        seen.body.contains("metadata store is unavailable"),
        "{}",
        seen.body
    );
    let request_id = seen.request_id.expect("x-amz-request-id");
    assert!(
        seen.body
            .contains(&format!("<RequestId>{request_id}</RequestId>")),
        "{}",
        seen.body
    );

    let late = send(state(NOW + 901, 64), request("GET", "/test.txt", &headers)).await;
    assert_eq!(
        (late.status, late.code.as_deref()),
        (403, Some("RequestTimeTooSkewed"))
    );
}

#[tokio::test]
async fn the_published_presigned_url_authenticates_and_reaches_its_handler() {
    let target = "/test.txt?X-Amz-Algorithm=AWS4-HMAC-SHA256\
        &X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request\
        &X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host\
        &X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404";
    let headers = [(
        "Host".to_owned(),
        "examplebucket.s3.amazonaws.com".to_owned(),
    )];
    let seen = send(state(NOW + 3600, 64), request("GET", target, &headers)).await;
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (503, Some("ServiceUnavailable")),
        "{}",
        seen.body
    );
}
