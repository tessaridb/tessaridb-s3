//! Server-side encryption on the wire: what a request asks for, and what a response answers. `AES256` seals under the
//! server's own key; `aws:kms` under a key an operator created by name — named by the request's key id, or the key
//! named `aws-s3` (the name of S3's own managed key) when it names none.

use axum::http::{HeaderMap, HeaderValue};
use tessari_s3_storage::encryption::{KeyName, Sealing};
use tessari_s3_storage::objects::{Content, StoredObject};
use tessari_s3_types::ErrorCode;

use super::headers::not_implemented;
use crate::error::{Error, Result};

const MODE: &str = "x-amz-server-side-encryption";
const KEY_ID: &str = "x-amz-server-side-encryption-aws-kms-key-id";
const BUCKET_KEY: &str = "x-amz-server-side-encryption-bucket-key-enabled";
/// The key `aws:kms` names when the request names none.
pub(crate) const DEFAULT_KEY: &str = "aws-s3";

/// The key a request's `aws-kms-key-id` names: the name itself, `alias/<name>`, or a KMS key or alias ARN ending in
/// one of those.
fn key_named(id: &str) -> Result<KeyName> {
    let tail = id
        .strip_prefix("arn:aws:kms:")
        .and_then(|arn| {
            arn.split_once(':')
                .and_then(|(_, rest)| rest.split_once(':'))
        })
        .map_or(id, |(_, resource)| resource);
    let name = tail
        .strip_prefix("key/")
        .or_else(|| tail.strip_prefix("alias/"))
        .unwrap_or(tail);
    // S3's own managed key is `alias/aws/s3`; it is the key named `aws-s3` here.
    let name = if name == "aws/s3" { DEFAULT_KEY } else { name };
    KeyName::new(name).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidArgument,
            "the KMS key id names no key this server could hold",
        )
    })
}

/// The encryption `headers` ask for: none, the server's key, or a named key.
///
/// # Errors
/// `InvalidArgument` for a mode S3 does not define or a key id without `aws:kms`; `NotImplemented` for dual-layer
/// KMS, an encryption context, and customer-provided keys.
pub(crate) fn read_sse(headers: &HeaderMap) -> Result<Option<Sealing>> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
    };
    for (name, _) in headers {
        let name = name.as_str();
        if name.starts_with("x-amz-server-side-encryption-customer-") {
            return Err(not_implemented(
                "server-side encryption with a customer key",
            ));
        }
        if name == "x-amz-server-side-encryption-context" {
            return Err(not_implemented("an encryption context"));
        }
        if name.starts_with("x-amz-server-side-encryption-") && name != KEY_ID && name != BUCKET_KEY
        {
            return Err(not_implemented(name));
        }
    }
    match (text(MODE), text(KEY_ID)) {
        (None, None) => Ok(None),
        (Some("AES256"), None) => Ok(Some(Sealing::Server)),
        (Some("aws:kms"), id) => Ok(Some(Sealing::Named(
            id.map_or_else(|| key_named(DEFAULT_KEY), key_named)?,
        ))),
        (Some("aws:kms:dsse"), _) => Err(not_implemented("dual-layer KMS encryption")),
        (None | Some("AES256"), Some(_)) => Err(Error::new(
            ErrorCode::InvalidArgument,
            "a KMS key id needs x-amz-server-side-encryption: aws:kms",
        )),
        (Some(other), _) => Err(Error::new(
            ErrorCode::InvalidArgument,
            format!("the server-side encryption {other} is not one S3 defines"),
        )),
    }
}

/// Sets the response headers that say how an object is sealed.
pub(crate) fn answer_sse(headers: &mut HeaderMap, sealing: &Sealing) {
    match sealing {
        Sealing::Server => {
            headers.insert(MODE, HeaderValue::from_static("AES256"));
        }
        Sealing::Named(name) => {
            headers.insert(MODE, HeaderValue::from_static("aws:kms"));
            if let Ok(value) = HeaderValue::from_str(name.as_str()) {
                headers.insert(KEY_ID, value);
            }
        }
    }
}

/// How `object` is sealed: its data's key, or its first sealed part's.
pub(crate) fn object_sealing(object: &StoredObject) -> Option<Sealing> {
    match &object.content {
        Content::Data(_) => object.encryption.as_ref().and_then(Sealing::of),
        Content::Parts(multipart) => multipart
            .parts
            .iter()
            .find_map(|part| part.encryption.as_ref())
            .and_then(Sealing::of),
    }
}

/// The quoted ETag of bytes whose MD5 is `md5_hex`: that MD5, or under a named key an opaque value — an MD5 of the
/// plaintext would tell anyone holding the metadata which sealed objects hold the same bytes.
///
/// # Errors
/// `InternalError` when the platform's generator fails.
pub(crate) fn etag_for(md5_hex: &str, sealing: Option<&Sealing>) -> Result<String> {
    if !matches!(sealing, Some(Sealing::Named(_))) {
        return Ok(format!("\"{md5_hex}\""));
    }
    let mut opaque = [0_u8; 16];
    getrandom::fill(&mut opaque).map_err(|_| {
        Error::new(
            ErrorCode::InternalError,
            "we encountered an internal error; please try again",
        )
    })?;
    let hex: String = opaque.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("\"{hex}\""))
}

#[cfg(test)]
#[path = "sse_tests.rs"]
mod tests;
