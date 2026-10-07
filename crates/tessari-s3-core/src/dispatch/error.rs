//! A request the dispatcher cannot resolve to exactly one operation.

use tessari_s3_types::ErrorCategory;

/// Why a request resolves to no operation. None of these ever falls through to a data-mutating handler.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DispatchError {
    /// The method is not modelled for this path shape and subresource.
    #[error("method not allowed for this resource")]
    MethodNotAllowed,
    /// A query parameter that no matching operation models.
    #[error("unknown query parameter {name}")]
    UnknownQueryParameter {
        /// The parameter's name.
        name: String,
    },
    /// The subresource selects an operation, but a parameter or header it requires is missing.
    #[error("{operation} requires {missing}")]
    MissingParameter {
        /// The operation the subresource selects.
        operation: &'static str,
        /// The missing query parameter or header.
        missing: &'static str,
    },
    /// Two operations match equally; a defect in the catalog, never a client error.
    #[error("ambiguous dispatch between {first} and {second}")]
    Ambiguous {
        /// One operation.
        first: &'static str,
        /// The other.
        second: &'static str,
    },
}

impl DispatchError {
    /// The S3 error code the request is answered with.
    #[must_use]
    pub const fn s3_code(&self) -> &'static str {
        match self {
            Self::MethodNotAllowed => "MethodNotAllowed",
            Self::UnknownQueryParameter { .. } => "InvalidArgument",
            Self::MissingParameter { .. } => "InvalidRequest",
            Self::Ambiguous { .. } => "InternalError",
        }
    }

    /// The taxonomy category.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::MethodNotAllowed
            | Self::UnknownQueryParameter { .. }
            | Self::MissingParameter { .. } => ErrorCategory::Validation,
            Self::Ambiguous { .. } => ErrorCategory::Internal,
        }
    }
}
