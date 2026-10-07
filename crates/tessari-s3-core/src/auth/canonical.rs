//! The SigV4 canonical request, rebuilt from the request exactly as it arrived on the wire.

use super::encode::{percent_decode, uri_encode};
use super::{AuthError, AuthResult};

/// The parts of an HTTP request SigV4 signs, as they arrived: the path and query are the raw request target, not a
/// framework's decoded and re-encoded URL, because canonicalising a normalised URL signs a different key than the
/// one written.
#[derive(Debug, Clone, Copy)]
pub struct SignedRequest<'a> {
    /// The HTTP method, e.g. `PUT`.
    pub method: &'a str,
    /// The path part of the request target, still percent-encoded, starting with `/`.
    pub raw_path: &'a str,
    /// The query part of the request target without the `?`, still percent-encoded.
    pub raw_query: &'a str,
    /// Every request header, in arrival order, names in any case.
    pub headers: &'a [(&'a str, &'a str)],
}

impl<'a> SignedRequest<'a> {
    /// The values of every header named `name` (case-insensitive), in arrival order.
    pub fn header_values<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'a str> + 's {
        self.headers
            .iter()
            .filter(move |(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
    }

    /// The first value of `name`, if present.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&'a str> {
        self.header_values(name).next()
    }
}

/// Builds the canonical request.
///
/// `exclude_query` names a query parameter left out (`X-Amz-Signature` for a presigned URL); `signed_headers` is
/// the client's SignedHeaders list, lowercase and sorted as parsed.
///
/// # Errors
/// [`AuthError::InvalidPercentEncoding`] for a broken escape; [`AuthError::SignedHeaderMissing`] when a signed
/// header is absent.
pub fn canonical_request(
    request: &SignedRequest<'_>,
    exclude_query: Option<&str>,
    signed_headers: &[String],
    payload_hash: &str,
) -> AuthResult<String> {
    let mut out = String::with_capacity(512);
    out.push_str(request.method);
    out.push('\n');
    canonical_uri(request.raw_path, &mut out)?;
    out.push('\n');
    canonical_query(request.raw_query, exclude_query, &mut out)?;
    out.push('\n');
    for name in signed_headers {
        let mut values = request
            .header_values(name)
            .map(canonical_header_value)
            .peekable();
        if values.peek().is_none() {
            return Err(AuthError::SignedHeaderMissing { name: name.clone() });
        }
        out.push_str(name);
        out.push(':');
        out.push_str(&values.collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&signed_headers.join(";"));
    out.push('\n');
    out.push_str(payload_hash);
    Ok(out)
}

/// The path decoded once and encoded once with `/` kept (S3 does not double-encode); an empty path is `/`.
fn canonical_uri(raw_path: &str, out: &mut String) -> AuthResult<()> {
    if raw_path.is_empty() {
        out.push('/');
        return Ok(());
    }
    uri_encode(&percent_decode(raw_path)?, true, out);
    Ok(())
}

/// Each name and value decoded once, encoded separately, then sorted by encoded name and value; a parameter
/// without `=` contributes `name=`.
fn canonical_query(raw_query: &str, exclude: Option<&str>, out: &mut String) -> AuthResult<()> {
    let mut pairs = Vec::new();
    for parameter in raw_query.split('&').filter(|p| !p.is_empty()) {
        let (raw_name, raw_value) = parameter.split_once('=').unwrap_or((parameter, ""));
        let name = percent_decode(raw_name)?;
        if exclude.is_some_and(|excluded| excluded.as_bytes() == name.as_slice()) {
            continue;
        }
        let (mut encoded_name, mut encoded_value) = (String::new(), String::new());
        uri_encode(&name, false, &mut encoded_name);
        uri_encode(&percent_decode(raw_value)?, false, &mut encoded_value);
        pairs.push((encoded_name, encoded_value));
    }
    pairs.sort_unstable();
    let joined: Vec<String> = pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    out.push_str(&joined.join("&"));
    Ok(())
}

/// A header value trimmed, with every run of spaces collapsed to one.
fn canonical_header_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Checks that every header the client must sign is signed: `host`, `content-type` when present (header
/// authentication), and every `x-amz-*` header present.
///
/// # Errors
/// [`AuthError::HeaderNotSigned`] naming the first unsigned one; [`AuthError::SignedHeaderMissing`] if `host` is
/// not even present.
pub fn check_required_signed(
    request: &SignedRequest<'_>,
    signed_headers: &[String],
    require_content_type: bool,
) -> AuthResult<()> {
    let signed = |name: &str| signed_headers.iter().any(|s| s == name);
    if request.header("host").is_none() {
        return Err(AuthError::SignedHeaderMissing {
            name: "host".to_owned(),
        });
    }
    for (name, _) in request.headers {
        let lower = name.to_ascii_lowercase();
        let must_sign = lower == "host"
            || lower.starts_with("x-amz-")
            || (require_content_type && lower == "content-type");
        if must_sign && !signed(&lower) {
            return Err(AuthError::HeaderNotSigned { name: lower });
        }
    }
    Ok(())
}
