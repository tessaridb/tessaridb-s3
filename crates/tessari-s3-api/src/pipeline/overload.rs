//! Load shedding: past the in-flight limit a request is answered `SlowDown` (503) at once, before any work, so
//! overload is signalled to clients that back off rather than queued until the process runs out of memory.

use axum::BoxError;
use axum::body::Body;
use axum::http::Response;
use tessari_s3_types::ErrorCode;

use super::response::{error_response, new_request_id};
use crate::Error;

/// The response for a request the service stack refused: `SlowDown` when shed, `InternalError` otherwise.
pub async fn refused(error: BoxError) -> Response<Body> {
    let request_id = new_request_id();
    let error = if error.is::<tower::load_shed::error::Overloaded>() {
        tracing::warn!(request_id = %request_id, "request shed: in-flight limit reached");
        Error::new(ErrorCode::SlowDown, "please reduce your request rate")
    } else {
        tracing::error!(request_id = %request_id, error = %error, "service stack failure");
        Error::new(ErrorCode::InternalError, "internal error")
    };
    error_response(&error, "", &request_id)
}
