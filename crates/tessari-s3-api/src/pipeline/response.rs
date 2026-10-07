//! Responses every request ends in: the `<Error>` envelope, the request id on every response, no body on HEAD.

use axum::body::Body;
use axum::http::{HeaderValue, Response, StatusCode, header};

use crate::Error;
use crate::xml::escape as xml_escape;

/// A fresh request id: 16 uppercase hex digits, the shape S3 clients log.
#[must_use]
pub fn new_request_id() -> String {
    let mut id = uuid::Uuid::new_v4().simple().to_string();
    id.truncate(16);
    id.make_ascii_uppercase();
    id
}

/// Adds the request id headers every response carries.
#[must_use]
pub fn with_request_id(mut response: Response<Body>, request_id: &str) -> Response<Body> {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        let headers = response.headers_mut();
        headers.insert("x-amz-request-id", value.clone());
        headers.insert("x-amz-id-2", value);
    }
    response
}

/// An XML document response with `status`.
#[must_use]
pub fn xml_response(status: StatusCode, document: String) -> Response<Body> {
    let mut response = Response::new(Body::from(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{document}"
    )));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}

/// An empty response with `status`.
#[must_use]
pub fn empty_response(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
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

#[cfg(test)]
mod tests {
    use super::new_request_id;

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
}
