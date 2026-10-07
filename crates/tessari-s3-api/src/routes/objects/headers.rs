//! What a PutObject's headers ask for: system headers to store, user metadata, the declared integrity values and the
//! write condition. A header that would change what is stored or who may read it, and that this server does not
//! implement, is refused — never accepted and dropped.

use std::collections::BTreeMap;

use axum::http::HeaderMap;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tessari_s3_constants::USER_METADATA_MAX;
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Declared};
use tessari_s3_storage::objects::WriteCondition;
use tessari_s3_types::ErrorCode;

use crate::{Error, Result};

/// System headers stored verbatim and returned on reads.
const STORED: [&str; 6] = [
    "cache-control",
    "content-disposition",
    "content-encoding",
    "content-language",
    "content-type",
    "expires",
];

/// The declared checksum: its algorithm, and its value when it came in a header (a trailer brings it at the end).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredChecksum {
    pub(crate) algorithm: ChecksumAlgorithm,
    pub(crate) header_value: Option<String>,
}

/// A PutObject's headers, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PutHeaders {
    pub(crate) system: BTreeMap<String, String>,
    pub(crate) metadata: BTreeMap<String, String>,
    pub(crate) integrity: Integrity,
    pub(crate) condition: WriteCondition,
}

fn not_implemented(what: &str) -> Error {
    Error::new(
        ErrorCode::NotImplemented,
        format!("{what} is not implemented"),
    )
}

/// Reads `headers` for a PutObject.
///
/// # Errors
/// `NotImplemented` for tagging, encryption, Object Lock, website redirects, appends and unsupported checksum
/// algorithms; `AccessControlListNotSupported`, `InvalidStorageClass`, `MetadataTooLarge`, `InvalidDigest`,
/// `InvalidRequest` and `InvalidArgument` for the rest.
pub(crate) fn read_put_headers(headers: &HeaderMap) -> Result<PutHeaders> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    };
    let mut system = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    let mut metadata_bytes: usize = 0;
    for (name, value) in headers {
        let name = name.as_str();
        let value = value
            .to_str()
            .map_err(|_| Error::new(ErrorCode::InvalidArgument, "a header is not ASCII"))?;
        if let Some(meta) = name.strip_prefix("x-amz-meta-") {
            metadata_bytes = metadata_bytes
                .saturating_add(meta.len())
                .saturating_add(value.len());
            metadata.insert(meta.to_owned(), value.to_owned());
        } else if STORED.contains(&name) {
            system.insert(name.to_owned(), value.to_owned());
        } else if name == "x-amz-tagging" {
            return Err(not_implemented("object tagging"));
        } else if name.starts_with("x-amz-server-side-encryption") {
            return Err(not_implemented("server-side encryption"));
        } else if name.starts_with("x-amz-object-lock-") {
            return Err(not_implemented("Object Lock"));
        } else if name == "x-amz-website-redirect-location" {
            return Err(not_implemented("a website redirect"));
        } else if name == "x-amz-write-offset-bytes" {
            return Err(not_implemented("appending to an object"));
        } else if name.starts_with("x-amz-grant-") {
            return Err(Error::new(
                ErrorCode::AccessControlListNotSupported,
                "the bucket does not allow ACLs",
            ));
        }
    }
    if metadata_bytes > USER_METADATA_MAX {
        return Err(Error::new(
            ErrorCode::MetadataTooLarge,
            "user metadata exceeds 2 KB",
        ));
    }
    if let Some(acl) =
        text("x-amz-acl").filter(|acl| !matches!(*acl, "private" | "bucket-owner-full-control"))
    {
        return Err(Error::new(
            ErrorCode::AccessControlListNotSupported,
            format!("the ACL {acl} is not allowed"),
        ));
    }
    if text("x-amz-storage-class").is_some_and(|class| class != "STANDARD") {
        return Err(Error::new(
            ErrorCode::InvalidStorageClass,
            "this server offers the STANDARD storage class only",
        ));
    }
    if let Some(encoding) = system.get_mut("content-encoding") {
        let kept: Vec<&str> = encoding
            .split(',')
            .map(str::trim)
            .filter(|c| !c.eq_ignore_ascii_case("aws-chunked"))
            .collect();
        *encoding = kept.join(", ");
    }
    system.retain(|_, value| !value.is_empty());
    let integrity = read_integrity(headers)?;
    let condition = match (text("if-none-match"), text("if-match")) {
        (None, None) => WriteCondition::None,
        (Some("*"), None) => WriteCondition::IfNoneMatch,
        (Some(_), None) => return Err(not_implemented("If-None-Match with a value other than *")),
        (None, Some(etag)) => WriteCondition::IfMatch(format!("\"{}\"", etag.trim_matches('"'))),
        (Some(_), Some(_)) => {
            return Err(Error::new(
                ErrorCode::InvalidArgument,
                "If-Match and If-None-Match cannot be combined",
            ));
        }
    };
    Ok(PutHeaders {
        system,
        metadata,
        integrity,
        condition,
    })
}

/// The integrity values a request with a body declares: its Content-MD5 and its one checksum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Integrity {
    pub(crate) content_md5: Option<[u8; 16]>,
    pub(crate) checksum: Option<DeclaredChecksum>,
}

/// Reads Content-MD5 and the declared checksum; a checksum header for an algorithm this server does not support is
/// refused rather than ignored.
pub(crate) fn read_integrity(headers: &HeaderMap) -> Result<Integrity> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    };
    let mut header_checksums = Vec::new();
    for (name, value) in headers {
        let name = name.as_str();
        if name.starts_with("x-amz-checksum-")
            && name != "x-amz-checksum-type"
            && name != "x-amz-checksum-mode"
        {
            let value = value
                .to_str()
                .map_err(|_| Error::new(ErrorCode::InvalidArgument, "a header is not ASCII"))?;
            match ChecksumAlgorithm::parse(name) {
                Some(Declared::Supported(algorithm)) => {
                    header_checksums.push((algorithm, value.trim().to_owned()));
                }
                _ => return Err(not_implemented(&format!("the checksum header {name}"))),
            }
        }
    }
    let checksum = declared_checksum(
        text("x-amz-sdk-checksum-algorithm"),
        text("x-amz-trailer"),
        header_checksums,
    )?;
    let content_md5 = text("content-md5")
        .map(|md5| {
            STANDARD
                .decode(md5)
                .ok()
                .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidDigest,
                        "Content-MD5 is not the base64 of 16 bytes",
                    )
                })
        })
        .transpose()?;
    Ok(Integrity {
        content_md5,
        checksum,
    })
}

/// The one checksum the request declares: by `x-amz-sdk-checksum-algorithm`, a checksum header, or `x-amz-trailer`.
fn declared_checksum(
    sdk: Option<&str>,
    trailer: Option<&str>,
    mut headers: Vec<(ChecksumAlgorithm, String)>,
) -> Result<Option<DeclaredChecksum>> {
    let conflicting = || {
        Error::new(
            ErrorCode::InvalidRequest,
            "the checksum headers name different algorithms",
        )
    };
    if headers.len() > 1 {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "more than one checksum header",
        ));
    }
    let from_header = headers.pop();
    let parse = |text: &str| match ChecksumAlgorithm::parse(text) {
        Some(Declared::Supported(algorithm)) => Ok(algorithm),
        _ => Err(not_implemented(&format!("the checksum algorithm {text}"))),
    };
    let from_trailer = trailer.map(parse).transpose()?;
    let from_sdk = sdk.map(parse).transpose()?;
    let algorithm = match (
        from_header.as_ref().map(|(a, _)| *a),
        from_trailer,
        from_sdk,
    ) {
        (None, None, None) => return Ok(None),
        (header, trailer, sdk) => {
            let named: Vec<ChecksumAlgorithm> =
                [header, trailer, sdk].into_iter().flatten().collect();
            let first = *named.first().ok_or_else(conflicting)?;
            if named.iter().any(|a| *a != first) {
                return Err(conflicting());
            }
            first
        }
    };
    Ok(Some(DeclaredChecksum {
        algorithm,
        header_value: from_header.map(|(_, value)| value),
    }))
}

#[cfg(test)]
#[path = "headers_tests.rs"]
mod tests;
