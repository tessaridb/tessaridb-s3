//! The internal surface's refusals: a fixed table of status, code and message, written as `{"code", "message"}`.

use axum::BoxError;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use tessari_s3_storage::Error as StorageError;

/// Why an internal request was not served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    /// No valid cluster signature.
    Unauthorized,
    /// A malformed id, index or parameter, or a body that is not the declared length.
    Invalid(&'static str),
    /// No such shard here.
    NotFound,
    /// The shard is being written by another request.
    Busy,
    /// The shard failed verification.
    Corrupt,
    /// Past the in-flight limit.
    Shed,
    /// The drive or the node failed.
    Internal,
}

impl Refusal {
    const fn parts(self) -> (StatusCode, &'static str, &'static str) {
        match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "the request is not signed by the cluster",
            ),
            Self::Invalid(reason) => (StatusCode::BAD_REQUEST, "invalid_request", reason),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "no such shard here"),
            Self::Busy => (
                StatusCode::CONFLICT,
                "shard_busy",
                "the shard is being written",
            ),
            Self::Corrupt => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "corrupt",
                "the shard failed verification",
            ),
            Self::Shed => (
                StatusCode::SERVICE_UNAVAILABLE,
                "slow_down",
                "too many requests in flight",
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "the node could not serve the request",
            ),
        }
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();
        // Every code and message is a literal of the table above (reasons included) with no quote, backslash or
        // control character, so the body is valid JSON without an encoder.
        let body = format!("{{\"code\":\"{code}\",\"message\":\"{message}\"}}");
        (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
    }
}

impl From<StorageError> for Refusal {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::InvalidShard { reason } => Self::Invalid(reason),
            StorageError::ShardLength { .. } => {
                Self::Invalid("the body is not the length the request declared")
            }
            StorageError::DataIo {
                kind: std::io::ErrorKind::NotFound,
                ..
            } => Self::NotFound,
            StorageError::DataIo {
                kind: std::io::ErrorKind::AlreadyExists,
                ..
            } => Self::Busy,
            StorageError::Corrupt { .. } => Self::Corrupt,
            other => {
                tracing::error!(error = %other, "internal shard request failed");
                Self::Internal
            }
        }
    }
}

/// The response for a request the service stack refused: shed past the in-flight limit, otherwise a failure.
pub(super) async fn shed(error: BoxError) -> Response {
    if error.is::<tower::load_shed::error::Overloaded>() {
        tracing::warn!("internal request shed: in-flight limit reached");
        Refusal::Shed.into_response()
    } else {
        tracing::error!(error = %error, "internal service stack failure");
        Refusal::Internal.into_response()
    }
}
