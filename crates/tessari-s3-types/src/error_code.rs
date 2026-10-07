//! The S3 error codes this server answers with, each with the HTTP status the core S3 error catalog gives it.
//!
//! Only catalog codes appear here: a client's retry and error handling branch on the code, so an invented code or a
//! near neighbour misroutes them.

/// An S3 error code from the core catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// 403: the request is not allowed.
    AccessDenied,
    /// 400: the body does not match its declared digest.
    BadDigest,
    /// 400: the body ended before its declared length.
    IncompleteBody,
    /// 500: a defect; never used for overload or validation.
    InternalError,
    /// 403: the access key is unknown.
    InvalidAccessKeyId,
    /// 400: an argument is invalid.
    InvalidArgument,
    /// 400: the bucket name breaks the naming rules.
    InvalidBucketName,
    /// 400: the request is invalid as a whole.
    InvalidRequest,
    /// 400: the URI cannot be parsed.
    InvalidUri,
    /// 400: the object key is longer than 1,024 bytes.
    KeyTooLongError,
    /// 405: the method is not allowed on this resource.
    MethodNotAllowed,
    /// 400: a required security header is missing.
    MissingSecurityHeader,
    /// 501: the operation is in the S3 API and this server does not implement it.
    NotImplemented,
    /// 403: the request time is too far from the server's.
    RequestTimeTooSkewed,
    /// 403: the signature does not match.
    SignatureDoesNotMatch,
    /// 503: slow down; the server is shedding load.
    SlowDown,
    /// 400: the signature algorithm is not supported.
    UnsupportedSignature,
}

impl ErrorCode {
    /// The code as it appears in `<Code>`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AccessDenied => "AccessDenied",
            Self::BadDigest => "BadDigest",
            Self::IncompleteBody => "IncompleteBody",
            Self::InternalError => "InternalError",
            Self::InvalidAccessKeyId => "InvalidAccessKeyId",
            Self::InvalidArgument => "InvalidArgument",
            Self::InvalidBucketName => "InvalidBucketName",
            Self::InvalidRequest => "InvalidRequest",
            Self::InvalidUri => "InvalidURI",
            Self::KeyTooLongError => "KeyTooLongError",
            Self::MethodNotAllowed => "MethodNotAllowed",
            Self::MissingSecurityHeader => "MissingSecurityHeader",
            Self::NotImplemented => "NotImplemented",
            Self::RequestTimeTooSkewed => "RequestTimeTooSkewed",
            Self::SignatureDoesNotMatch => "SignatureDoesNotMatch",
            Self::SlowDown => "SlowDown",
            Self::UnsupportedSignature => "UnsupportedSignature",
        }
    }

    /// The HTTP status the catalog gives the code.
    #[must_use]
    pub const fn http_status(self) -> u16 {
        match self {
            Self::AccessDenied
            | Self::InvalidAccessKeyId
            | Self::RequestTimeTooSkewed
            | Self::SignatureDoesNotMatch => 403,
            Self::BadDigest
            | Self::IncompleteBody
            | Self::InvalidArgument
            | Self::InvalidBucketName
            | Self::InvalidRequest
            | Self::InvalidUri
            | Self::KeyTooLongError
            | Self::MissingSecurityHeader
            | Self::UnsupportedSignature => 400,
            Self::InternalError => 500,
            Self::MethodNotAllowed => 405,
            Self::NotImplemented => 501,
            Self::SlowDown => 503,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ErrorCode;

    /// Every code against the catalog's `Code — HTTP status` row (S3 skill ref 02 M6, retrieved 2026-10-07).
    #[test]
    fn every_code_has_its_catalog_status() {
        let catalog = [
            (ErrorCode::AccessDenied, "AccessDenied", 403),
            (ErrorCode::BadDigest, "BadDigest", 400),
            (ErrorCode::IncompleteBody, "IncompleteBody", 400),
            (ErrorCode::InternalError, "InternalError", 500),
            (ErrorCode::InvalidAccessKeyId, "InvalidAccessKeyId", 403),
            (ErrorCode::InvalidArgument, "InvalidArgument", 400),
            (ErrorCode::InvalidBucketName, "InvalidBucketName", 400),
            (ErrorCode::InvalidRequest, "InvalidRequest", 400),
            (ErrorCode::InvalidUri, "InvalidURI", 400),
            (ErrorCode::KeyTooLongError, "KeyTooLongError", 400),
            (ErrorCode::MethodNotAllowed, "MethodNotAllowed", 405),
            (
                ErrorCode::MissingSecurityHeader,
                "MissingSecurityHeader",
                400,
            ),
            (ErrorCode::NotImplemented, "NotImplemented", 501),
            (ErrorCode::RequestTimeTooSkewed, "RequestTimeTooSkewed", 403),
            (
                ErrorCode::SignatureDoesNotMatch,
                "SignatureDoesNotMatch",
                403,
            ),
            (ErrorCode::SlowDown, "SlowDown", 503),
            (ErrorCode::UnsupportedSignature, "UnsupportedSignature", 400),
        ];
        for (code, text, status) in catalog {
            assert_eq!(
                (code.as_str(), code.http_status()),
                (text, status),
                "{code:?}"
            );
        }
    }
}
