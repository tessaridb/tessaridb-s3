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
    /// Response headers the error is answered with besides the envelope's own (a delete marker's, for one).
    pub headers: Vec<(&'static str, String)>,
}

impl Error {
    /// An error with `code` and `message`.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            headers: Vec::new(),
        }
    }

    /// The same error, answered with the header `name: value` as well.
    #[must_use]
    pub fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
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
            Storage::Meta(MetaError::Refused {
                class: Some(RefusalClass::Throttled),
                ..
            }) => {
                tracing::warn!(error = %error, "metadata store throttled the request");
                Self::new(
                    ErrorCode::SlowDown,
                    "the metadata store is busy; reduce your request rate",
                )
            }
            Storage::KeysSealed => Self::new(
                ErrorCode::ServiceUnavailable,
                "the encryption keys are sealed on this node; an operator must unseal them",
            ),
            Storage::DriveFull { .. } => {
                tracing::warn!(error = %error, "write refused: data drive below its free-space floor");
                Self::new(
                    ErrorCode::ServiceUnavailable,
                    "this node's data drive is below its free-space floor; writes are refused, reads are served",
                )
            }
            Storage::CustomerKeyMismatch => Self::new(
                ErrorCode::AccessDenied,
                "the customer key does not match the one the object was encrypted with",
            ),
            Storage::UnknownKey { name } => Self::new(
                ErrorCode::KmsNotFound,
                format!("no KMS key named {name} exists on this server"),
            ),
            Storage::OtherRegion { region } => Self::new(
                ErrorCode::PermanentRedirect,
                "the bucket you are attempting to access must be addressed using the specified endpoint",
            )
            .with_header("x-amz-bucket-region", region.clone()),
            Storage::Quorum { .. }
            | Storage::Unreadable { .. }
            | Storage::NoLayout
            | Storage::Peer(_) => {
                tracing::warn!(error = %error, "too few storage nodes reachable");
                Self::new(
                    ErrorCode::ServiceUnavailable,
                    "too few storage nodes are reachable; retry",
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

#[cfg(test)]
mod tests {
    use tessari_s3_infrastructure::tessaridb::{MetaError, RefusalClass};
    use tessari_s3_types::ErrorCode;

    use super::Error;

    fn refused(class: RefusalClass) -> Error {
        Error::from(tessari_s3_storage::Error::Meta(MetaError::Refused {
            message: "this node is not taking a sign-in for that user right now".to_owned(),
            class: Some(class),
        }))
    }

    #[test]
    fn a_throttled_metadata_store_answers_slow_down_so_clients_back_off_and_retry() {
        assert_eq!(refused(RefusalClass::Throttled).code, ErrorCode::SlowDown);
        assert_eq!(
            refused(RefusalClass::Retry).code,
            ErrorCode::ServiceUnavailable,
            "control: a retry-class refusal keeps its answer"
        );
    }

    #[test]
    fn a_drive_below_its_floor_answers_service_unavailable_never_an_internal_error() {
        let error = Error::from(tessari_s3_storage::Error::DriveFull {
            available: 10,
            floor: 20,
        });
        assert_eq!(error.code, ErrorCode::ServiceUnavailable);
    }
}
