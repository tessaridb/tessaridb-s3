//! Why a request failed authentication, and the S3 error code each reason is answered with.
//!
//! Codes come from the core S3 error catalog only. Where AWS answers with a code outside that catalog, the closest
//! catalog code is used and named here, so a client's retry logic sees a code it knows.

use tessari_s3_types::{ErrorCategory, ErrorCode};

/// A request that does not authenticate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// The `Authorization` header or the presign query is not the SigV4 shape.
    #[error("malformed SigV4 authorization: {reason}")]
    Malformed {
        /// Which part is malformed.
        reason: &'static str,
    },
    /// An algorithm other than `AWS4-HMAC-SHA256`.
    #[error("unsupported signature algorithm")]
    UnsupportedAlgorithm,
    /// A `%` escape in the request target that is not `%` and two hex digits.
    #[error("invalid percent-encoding in the request target")]
    InvalidPercentEncoding,
    /// `x-amz-date` (or `X-Amz-Date`) is missing or not `YYYYMMDDTHHMMSSZ`.
    #[error("missing or invalid request date")]
    InvalidDate,
    /// The credential scope names another date, region or service than this request and server.
    #[error("credential scope does not match: {reason}")]
    ScopeMismatch {
        /// Which component differs.
        reason: &'static str,
    },
    /// A header listed in SignedHeaders is absent from the request.
    #[error("signed header {name} is not present")]
    SignedHeaderMissing {
        /// The header's lowercase name.
        name: String,
    },
    /// A header that must be signed (`host`, `content-type`, any `x-amz-*`) is not in SignedHeaders.
    #[error("header {name} is present but not signed")]
    HeaderNotSigned {
        /// The header's lowercase name.
        name: String,
    },
    /// `x-amz-content-sha256` is missing from a header-signed request.
    #[error("x-amz-content-sha256 is required")]
    MissingContentSha256,
    /// The request time is more than 15 minutes from the server clock.
    #[error("request time is outside the allowed clock skew")]
    ClockSkew,
    /// `X-Amz-Expires` is outside 1-604800 seconds.
    #[error("X-Amz-Expires must be between 1 and 604800 seconds")]
    PresignExpiresOutOfRange,
    /// The presigned URL has expired, or is not valid yet.
    #[error("presigned URL is expired or not yet valid")]
    PresignExpired,
    /// The computed signature differs from the one supplied.
    #[error("signature does not match")]
    SignatureMismatch,
    /// The body's SHA-256 differs from the declared `x-amz-content-sha256`.
    #[error("the body does not match x-amz-content-sha256")]
    PayloadHashMismatch,
    /// An aws-chunked frame is malformed, short before the last chunk, or the stream ends early.
    #[error("malformed aws-chunked body: {reason}")]
    MalformedChunk {
        /// What is wrong with the frame.
        reason: &'static str,
    },
    /// The decoded chunk sizes do not add up to `x-amz-decoded-content-length`.
    #[error("decoded length does not match x-amz-decoded-content-length")]
    DecodedLengthMismatch,
}

impl AuthError {
    /// The S3 error code the request is answered with.
    #[must_use]
    pub const fn s3_code(&self) -> ErrorCode {
        match self {
            // AWS answers AuthorizationHeaderMalformed / AuthorizationQueryParametersError, outside the core catalog.
            Self::Malformed { .. }
            | Self::InvalidPercentEncoding
            | Self::PresignExpiresOutOfRange => ErrorCode::InvalidArgument,
            Self::UnsupportedAlgorithm => ErrorCode::UnsupportedSignature,
            Self::InvalidDate | Self::HeaderNotSigned { .. } | Self::PresignExpired => {
                ErrorCode::AccessDenied
            }
            Self::ScopeMismatch { .. }
            | Self::SignedHeaderMissing { .. }
            | Self::SignatureMismatch => ErrorCode::SignatureDoesNotMatch,
            Self::MissingContentSha256 => ErrorCode::MissingSecurityHeader,
            Self::ClockSkew => ErrorCode::RequestTimeTooSkewed,
            // AWS answers XAmzContentSHA256Mismatch, outside the core catalog.
            Self::PayloadHashMismatch => ErrorCode::BadDigest,
            // AWS answers InvalidChunkSizeError for a short chunk, outside the core catalog.
            Self::MalformedChunk { .. } => ErrorCode::InvalidRequest,
            Self::DecodedLengthMismatch => ErrorCode::IncompleteBody,
        }
    }

    /// The taxonomy category: a request that does not parse is `validation`, one that parses and does not verify
    /// is `unauthorized`.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::Malformed { .. }
            | Self::UnsupportedAlgorithm
            | Self::InvalidPercentEncoding
            | Self::PresignExpiresOutOfRange
            | Self::MissingContentSha256
            | Self::MalformedChunk { .. }
            | Self::DecodedLengthMismatch => ErrorCategory::Validation,
            Self::InvalidDate
            | Self::ScopeMismatch { .. }
            | Self::SignedHeaderMissing { .. }
            | Self::HeaderNotSigned { .. }
            | Self::ClockSkew
            | Self::PresignExpired
            | Self::SignatureMismatch
            | Self::PayloadHashMismatch => ErrorCategory::Unauthorized,
        }
    }
}

/// The result of an authentication step.
pub type AuthResult<T> = std::result::Result<T, AuthError>;
