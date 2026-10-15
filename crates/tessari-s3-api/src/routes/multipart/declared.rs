//! The checksum an upload declares at CreateMultipartUpload: an algorithm this server supports, and a type that
//! algorithm allows.

use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Declared};
use tessari_s3_core::objects::multipart::ObjectChecksum;
use tessari_s3_types::ErrorCode;

use crate::pipeline::call::Call;
use crate::{Error, Result};

/// The checksum declaration of a new upload: an algorithm this server supports, and a type it knows.
pub(super) fn declared_checksum(call: &Call<'_>) -> Result<(Option<String>, Option<String>)> {
    let header = |name: &str| call.headers.get(name).and_then(|v| v.to_str().ok());
    let algorithm = match header("x-amz-checksum-algorithm") {
        None => None,
        Some(text) => match ChecksumAlgorithm::parse(text) {
            Some(Declared::Supported(algorithm)) => Some(algorithm.name().to_owned()),
            _ => {
                return Err(Error::new(
                    ErrorCode::NotImplemented,
                    format!("the checksum algorithm {text} is not implemented"),
                ));
            }
        },
    };
    let kind = match header("x-amz-checksum-type") {
        None => None,
        Some(kind @ ("COMPOSITE" | "FULL_OBJECT")) => Some(kind.to_owned()),
        Some(_) => {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "x-amz-checksum-type must be COMPOSITE or FULL_OBJECT",
            ));
        }
    };
    let named = match algorithm.as_deref().map(ChecksumAlgorithm::parse) {
        Some(Some(Declared::Supported(algorithm))) => Some(algorithm),
        _ => None,
    };
    if ObjectChecksum::declared(named, kind.as_deref()).is_none() {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "the checksum type is not one the checksum algorithm allows",
        ));
    }
    Ok((algorithm, kind))
}
