//! Responses every request ends in: the `<Error>` envelope, the request id on every response, no body on HEAD.

use axum::body::Body;
use axum::http::{HeaderValue, Response, StatusCode, header};

use crate::Error;

/// A fresh request id: 16 uppercase hex digits, the shape S3 clients log.
#[must_use]
pub fn new_request_id() -> String {
    let mut id = uuid::Uuid::new_v4().simple().to_string();
    id.truncate(16);
    id.make_ascii_uppercase();
    id
}

/// Renders `error` for the request at `resource`. On a HEAD request the server strips the body and keeps the status
/// and headers, which is the HEAD contract clients rely on.
#[must_use]
pub fn error_response(error: &Error, resource: &str, request_id: &str) -> Response<Body> {
    let status =
        StatusCode::from_u16(error.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = Body::from(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>{}</Code><Message>{}</Message>\
         <Resource>{}</Resource><RequestId>{}</RequestId></Error>",
        error.code.as_str(),
        xml_escape(&error.message),
        xml_escape(resource),
        xml_escape(request_id),
    ));
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.insert("x-amz-request-id", value.clone());
        headers.insert("x-amz-id-2", value);
    }
    response
}

/// Escapes the five XML special characters.
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{new_request_id, xml_escape};

    #[test]
    fn request_ids_are_sixteen_uppercase_hex_digits_and_differ() {
        let (first, second) = (new_request_id(), new_request_id());
        assert_eq!(first.len(), 16);
        assert!(
            first
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b)),
            "{first}"
        );
        assert_ne!(first, second);
    }

    #[test]
    fn markup_in_a_key_cannot_escape_the_envelope() {
        assert_eq!(xml_escape("a<b>&\"c'"), "a&lt;b&gt;&amp;&quot;c&apos;");
    }
}
