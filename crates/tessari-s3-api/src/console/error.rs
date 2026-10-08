//! The console API's error envelope: `{"code", "message"}` with a stable snake_case code and the HTTP status that
//! goes with it. A refusal says what kind of refusal it is and nothing about the server's internals.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// A console request refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConsoleError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

#[derive(Serialize)]
struct Body {
    code: &'static str,
    message: &'static str,
}

impl ConsoleError {
    pub(crate) const fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "sign in to the console",
        }
    }

    pub(crate) const fn forbidden() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: "this key may not do that",
        }
    }

    pub(crate) const fn rate_limited() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "rate_limit",
            message: "too many requests; wait a minute",
        }
    }

    pub(crate) const fn not_json() -> Self {
        Self {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            code: "unsupported_media_type",
            message: "the body must be application/json",
        }
    }

    pub(crate) const fn bad_request(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message,
        }
    }

    pub(crate) const fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "no such console route",
        }
    }

    pub(crate) const fn invalid(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message,
        }
    }

    pub(crate) const fn missing(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code,
            message,
        }
    }

    pub(crate) const fn conflict(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code,
            message,
        }
    }

    pub(crate) const fn precondition_failed() -> Self {
        Self {
            status: StatusCode::PRECONDITION_FAILED,
            code: "precondition_failed",
            message: "the object changed since it was read; reload it",
        }
    }

    pub(crate) const fn unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "unavailable",
            message: "the metadata store or the data nodes did not answer; try again",
        }
    }

    pub(crate) const fn not_recorded() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "action_not_recorded",
            message: "the action was carried out but could not be recorded; it is in the server log",
        }
    }

    pub(crate) const fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal",
            message: "the console could not complete the request",
        }
    }
}

impl IntoResponse for ConsoleError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(Body {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

impl From<tessari_s3_storage::Error> for ConsoleError {
    /// A storage failure keeps its kind and loses its detail: the store being unreachable is `503`, anything else is
    /// `500`, and the detail goes to the log rather than to the browser.
    fn from(error: tessari_s3_storage::Error) -> Self {
        tracing::error!(error = %error, "console request failed in storage");
        if error.category() == tessari_s3_types::ErrorCategory::Unavailable {
            Self::unavailable()
        } else {
            Self::internal()
        }
    }
}
