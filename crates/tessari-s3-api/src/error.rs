//! The S3 error a request is answered with: a catalog code, a message and the resource it concerns.

use tessari_s3_core::auth::AuthError;
use tessari_s3_core::dispatch::DispatchError;
use tessari_s3_types::{ErrorCategory, ErrorCode};

/// An S3 error, rendered as the `<Error>` XML envelope with the code's catalog status.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {message}", code.as_str())]
pub struct Error {
    /// The catalog code; it decides the HTTP status and the client's retry behaviour.
    pub code: ErrorCode,
    /// A human-readable explanation; never a secret, a signature or internal detail.
    pub message: String,
}

impl Error {
    /// An error with `code` and `message`.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The taxonomy category of the code.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self.code.http_status() {
            403 => ErrorCategory::Unauthorized,
            500 => ErrorCategory::Internal,
            503 => ErrorCategory::Unavailable,
            _ => ErrorCategory::Validation,
        }
    }
}

impl From<AuthError> for Error {
    fn from(error: AuthError) -> Self {
        Self::new(error.s3_code(), error.to_string())
    }
}

impl From<tessari_s3_storage::Error> for Error {
    fn from(error: tessari_s3_storage::Error) -> Self {
        use tessari_s3_infrastructure::tessaridb::{MetaError, RefusalClass};
        use tessari_s3_storage::Error as Storage;
        match &error {
            Storage::Meta(MetaError::Unavailable { .. }) => {
                tracing::warn!(error = %error, "metadata store unavailable");
                Self::new(
                    ErrorCode::ServiceUnavailable,
                    "the metadata store is unavailable; retry",
                )
            }
            Storage::Meta(MetaError::Refused {
                class: Some(RefusalClass::Retry | RefusalClass::Unavailable),
                ..
            })
            | Storage::Contended => {
                tracing::warn!(error = %error, "metadata store asked for a retry");
                Self::new(
                    ErrorCode::ServiceUnavailable,
                    "the metadata store is busy; retry",
                )
            }
            _ => {
                tracing::error!(error = %error, "storage failure");
                Self::new(
                    ErrorCode::InternalError,
                    "we encountered an internal error; please try again",
                )
            }
        }
    }
}

impl From<DispatchError> for Error {
    fn from(error: DispatchError) -> Self {
        Self::new(error.s3_code(), error.to_string())
    }
}

/// The result of a pipeline step.
pub type Result<T> = std::result::Result<T, Error>;
