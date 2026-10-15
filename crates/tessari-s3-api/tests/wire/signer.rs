//! A SigV4 signer written for the tests, independently of the server's verifier: a request it signs and the
//! server accepts is evidence that both agree with the protocol, not that one agrees with itself.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// SHA-256 of the empty body.
pub(crate) const EMPTY_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac(key: &[u8], message: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    mac.update(message.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// What to sign. `path` is already encoded as it will go on the wire; query values are plain text.
pub(crate) struct Unsigned<'a> {
    pub(crate) method: &'a str,
    pub(crate) host: &'a str,
    pub(crate) path: &'a str,
    pub(crate) query: Vec<(&'a str, Option<&'a str>)>,
    pub(crate) headers: Vec<(&'a str, &'a str)>,
    pub(crate) access_key: &'a str,
    pub(crate) secret: &'a str,
    pub(crate) region: &'a str,
    pub(crate) amz_date: &'a str,
}

/// The request target (`path?query`) and every header including `Authorization`, for an empty body.
pub(crate) fn sign(request: &Unsigned<'_>) -> (String, Vec<(String, String)>) {
    sign_with_body(request, b"")
}

/// As [`sign`], declaring `body` by its SHA-256.
pub(crate) fn sign_with_body(
    request: &Unsigned<'_>,
    body: &[u8],
) -> (String, Vec<(String, String)>) {
    let payload_hash = hex(&Sha256::digest(body));
    let mut query: Vec<(String, String)> = request
        .query
        .iter()
        .map(|(name, value)| (encode(name), encode(value.unwrap_or(""))))
        .collect();
    query.sort();
    let canonical_query: Vec<String> = query.iter().map(|(n, v)| format!("{n}={v}")).collect();
    let canonical_query = canonical_query.join("&");
    let wire_query: Vec<String> = request
        .query
        .iter()
        .map(|(name, value)| match value {
            Some(value) => format!("{}={}", encode(name), encode(value)),
            None => encode(name),
        })
        .collect();
    let target = if wire_query.is_empty() {
        request.path.to_owned()
    } else {
        format!("{}?{}", request.path, wire_query.join("&"))
    };
    let mut headers: Vec<(String, String)> = vec![
        ("host".to_owned(), request.host.to_owned()),
        ("x-amz-date".to_owned(), request.amz_date.to_owned()),
        ("x-amz-content-sha256".to_owned(), payload_hash.clone()),
    ];
    headers.extend(
        request
            .headers
            .iter()
            .map(|(n, v)| (n.to_ascii_lowercase(), (*v).to_owned())),
    );
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(n, v)| format!("{n}:{v}\n")).collect();
    let signed_headers: Vec<&str> = headers.iter().map(|(n, _)| n.as_str()).collect();
    let signed_headers = signed_headers.join(";");
    let canonical_request = format!(
        "{}\n{}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        request.method, request.path
    );
    let date = &request.amz_date[..8];
    let scope = format!("{date}/{}/s3/aws4_request", request.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        request.amz_date,
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let mut key = hmac(format!("AWS4{}", request.secret).as_bytes(), date);
    for part in [request.region, "s3", "aws4_request"] {
        key = hmac(&key, part);
    }
    let signature = hex(&hmac(&key, &string_to_sign));
    headers.push((
        "authorization".to_owned(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope},SignedHeaders={signed_headers},Signature={signature}",
            request.access_key
        ),
    ));
    (target, headers)
}
