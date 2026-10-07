//! The S3 error codes this server answers with, each with the HTTP status the core S3 error catalog gives it.
//!
//! Only catalog codes appear here: a client's retry and error handling branch on the code, so an invented code or a
//! near neighbour misroutes them.

/// An S3 error code from the core catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// 400: the bucket does not allow ACLs.
    AccessControlListNotSupported,
    /// 400: the Content-MD5 or checksum value is not well formed.
    InvalidDigest,
    /// 400: the upload exceeds the maximum allowed object size.
    EntityTooLarge,
    /// 400: the storage class is not one this server offers.
    InvalidStorageClass,
    /// 411: the request needs a Content-Length.
    MissingContentLength,
    /// 400: user metadata exceeds 2 KB.
    MetadataTooLarge,
    /// 416: the requested range cannot be satisfied.
    InvalidRange,
    /// 304: the object has not changed since the condition named.
    NotModified,
    /// 412: a condition the request named does not hold.
    PreconditionFailed,
    /// 404: the key holds no object.
    NoSuchKey,
    /// The multipart upload does not exist, or was completed or aborted.
    NoSuchUpload,
    /// 503: a dependency the request needs is unavailable; retry.
    ServiceUnavailable,
    /// 404: the bucket does not exist.
    NoSuchBucket,
    /// 400: the request body is longer than this operation accepts.
    MaxMessageLengthExceeded,
    /// 400: the XML body is not well formed or not the expected shape.
    MalformedXml,
    /// 400: the bucket would be created in another region than this server's.
    IllegalLocationConstraintException,
    /// 409: the bucket still holds objects.
    BucketNotEmpty,
    /// 409: the bucket exists and the caller owns it.
    BucketAlreadyOwnedByYou,
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
            Self::AccessControlListNotSupported => "AccessControlListNotSupported",
            Self::InvalidDigest => "InvalidDigest",
            Self::EntityTooLarge => "EntityTooLarge",
            Self::InvalidStorageClass => "InvalidStorageClass",
            Self::MissingContentLength => "MissingContentLength",
            Self::MetadataTooLarge => "MetadataTooLarge",
            Self::InvalidRange => "InvalidRange",
            Self::NotModified => "NotModified",
            Self::PreconditionFailed => "PreconditionFailed",
            Self::NoSuchKey => "NoSuchKey",
            Self::NoSuchUpload => "NoSuchUpload",
            Self::ServiceUnavailable => "ServiceUnavailable",
            Self::NoSuchBucket => "NoSuchBucket",
            Self::MaxMessageLengthExceeded => "MaxMessageLengthExceeded",
            Self::MalformedXml => "MalformedXML",
            Self::IllegalLocationConstraintException => "IllegalLocationConstraintException",
            Self::BucketNotEmpty => "BucketNotEmpty",
            Self::BucketAlreadyOwnedByYou => "BucketAlreadyOwnedByYou",
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
            Self::BucketAlreadyOwnedByYou | Self::BucketNotEmpty => 409,
            Self::IllegalLocationConstraintException
            | Self::MalformedXml
            | Self::MaxMessageLengthExceeded => 400,
            Self::NoSuchBucket => 404,
            Self::ServiceUnavailable => 503,
            Self::NoSuchKey | Self::NoSuchUpload => 404,
            Self::PreconditionFailed => 412,
            Self::NotModified => 304,
            Self::InvalidRange => 416,
            Self::MissingContentLength => 411,
            Self::MetadataTooLarge
            | Self::InvalidStorageClass
            | Self::InvalidDigest
            | Self::EntityTooLarge
            | Self::AccessControlListNotSupported => 400,
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
            (
                ErrorCode::AccessControlListNotSupported,
                "AccessControlListNotSupported",
                400,
            ),
            (ErrorCode::InvalidDigest, "InvalidDigest", 400),
            (ErrorCode::InvalidStorageClass, "InvalidStorageClass", 400),
            (ErrorCode::MissingContentLength, "MissingContentLength", 411),
            (ErrorCode::MetadataTooLarge, "MetadataTooLarge", 400),
            (ErrorCode::InvalidRange, "InvalidRange", 416),
            (ErrorCode::NotModified, "NotModified", 304),
            (ErrorCode::PreconditionFailed, "PreconditionFailed", 412),
            (ErrorCode::NoSuchKey, "NoSuchKey", 404),
            (ErrorCode::NoSuchUpload, "NoSuchUpload", 404),
            (ErrorCode::ServiceUnavailable, "ServiceUnavailable", 503),
            (ErrorCode::NoSuchBucket, "NoSuchBucket", 404),
            (
                ErrorCode::MaxMessageLengthExceeded,
                "MaxMessageLengthExceeded",
                400,
            ),
            (ErrorCode::MalformedXml, "MalformedXML", 400),
            (
                ErrorCode::IllegalLocationConstraintException,
                "IllegalLocationConstraintException",
                400,
            ),
            (ErrorCode::BucketNotEmpty, "BucketNotEmpty", 409),
            (
                ErrorCode::BucketAlreadyOwnedByYou,
                "BucketAlreadyOwnedByYou",
                409,
            ),
            (ErrorCode::BadDigest, "BadDigest", 400),
            (ErrorCode::EntityTooLarge, "EntityTooLarge", 400),
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
