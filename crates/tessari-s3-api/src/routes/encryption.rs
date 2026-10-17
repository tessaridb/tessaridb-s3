//! GetBucketEncryption, PutBucketEncryption and DeleteBucketEncryption (S3 ref 11 M3). A bucket's default encryption
//! seals every write that asks for none — PutObject, CreateMultipartUpload, CopyObject — under the server's key or a
//! named one; a write that asks for its own encryption keeps it. A default is accepted only when this node can seal
//! under it now: its keys unsealed and, under `aws:kms`, the named key held. Writes bringing their own key (SSE-C) stay
//! blocked unless `BlockedEncryptionTypes` names `NONE`, as on AWS since April 2026.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_storage::buckets::BucketEncryption;
use tessari_s3_storage::encryption::Sealing;
use tessari_s3_types::ErrorCode;

use super::buckets::{existing, no_such_bucket};
use super::objects::sse::{DEFAULT_KEY, key_named};
use super::xml_tree::{Node, malformed, single, tree};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::S3_NAMESPACE;
use crate::{Error, Result};

const ROOT: &str = "ServerSideEncryptionConfiguration";
/// configuration › rule › default › algorithm.
const MAX_DEPTH: usize = 4;

/// The configuration document a bucket answers with.
pub(super) fn document(encryption: &BucketEncryption) -> String {
    let by_default = match &encryption.sealing {
        Sealing::Server => "<SSEAlgorithm>AES256</SSEAlgorithm>".to_owned(),
        Sealing::Named(name) => format!(
            "<SSEAlgorithm>aws:kms</SSEAlgorithm><KMSMasterKeyID>{}</KMSMasterKeyID>",
            name.as_str()
        ),
    };
    let blocked = if encryption.allow_customer_keys {
        "NONE"
    } else {
        "SSE-C"
    };
    format!(
        "<{ROOT} xmlns=\"{S3_NAMESPACE}\"><Rule><ApplyServerSideEncryptionByDefault>{by_default}\
         </ApplyServerSideEncryptionByDefault><BucketKeyEnabled>{}</BucketKeyEnabled><BlockedEncryptionTypes>\
         <EncryptionType>{blocked}</EncryptionType></BlockedEncryptionTypes></Rule></{ROOT}>",
        encryption.bucket_key
    )
}

/// The sealing an `ApplyServerSideEncryptionByDefault` element names.
fn sealing(node: &Node) -> Result<Sealing> {
    let (mut algorithm, mut key) = (None, None);
    for child in &node.children {
        match child.name.as_str() {
            "SSEAlgorithm" => single(&mut algorithm, child)?,
            "KMSMasterKeyID" => single(&mut key, child)?,
            _ => return Err(malformed()),
        }
    }
    let key = key.map(|node| node.text.trim());
    match (algorithm.map(|node| node.text.trim()), key) {
        (Some("AES256"), None) => Ok(Sealing::Server),
        (Some("AES256"), Some(_)) => Err(Error::new(
            ErrorCode::InvalidArgument,
            "a KMSMasterKeyID needs the SSEAlgorithm aws:kms",
        )),
        (Some("aws:kms"), key) => Ok(Sealing::Named(key_named(key.unwrap_or(DEFAULT_KEY))?)),
        (Some("aws:kms:dsse"), _) => Err(Error::new(
            ErrorCode::NotImplemented,
            "dual-layer KMS encryption is not supported",
        )),
        _ => Err(malformed()),
    }
}

/// Whether a `BlockedEncryptionTypes` allows customer keys: one `EncryptionType`, `NONE` allowing them and `SSE-C`
/// blocking them.
fn allows_customer_keys(node: &Node) -> Result<bool> {
    let mut named = None;
    for child in &node.children {
        match child.name.as_str() {
            "EncryptionType" => single(&mut named, child)?,
            _ => return Err(malformed()),
        }
    }
    match named.map(|node| node.text.trim()) {
        Some("NONE") => Ok(true),
        Some("SSE-C") => Ok(false),
        _ => Err(malformed()),
    }
}

/// The default encryption `document` states.
///
/// # Errors
/// `MalformedXML` for a document that is not the shape, or not exactly one rule; `InvalidArgument` for a key id
/// without `aws:kms`; `NotImplemented` for dual-layer KMS.
pub(super) fn parse(document: &[u8]) -> Result<BucketEncryption> {
    let root = tree(document, MAX_DEPTH)?;
    let mut rule = None;
    for child in &root.children {
        match child.name.as_str() {
            "Rule" if root.name == ROOT => single(&mut rule, child)?,
            _ => return Err(malformed()),
        }
    }
    let rule = rule.ok_or_else(malformed)?;
    let (mut by_default, mut bucket_key, mut types) = (None, None, None);
    for child in &rule.children {
        match child.name.as_str() {
            "ApplyServerSideEncryptionByDefault" => single(&mut by_default, child)?,
            "BucketKeyEnabled" => single(&mut bucket_key, child)?,
            "BlockedEncryptionTypes" => single(&mut types, child)?,
            _ => return Err(malformed()),
        }
    }
    let sealing = sealing(by_default.ok_or_else(malformed)?)?;
    let allow_customer_keys = types
        .map(allows_customer_keys)
        .transpose()?
        .unwrap_or(false);
    let bucket_key = match bucket_key.map(|node| node.text.trim()) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(malformed()),
    };
    Ok(BucketEncryption {
        sealing,
        bucket_key,
        allow_customer_keys,
    })
}

/// `GET /{bucket}?encryption`.
pub(crate) async fn get(call: &Call<'_>) -> Result<Response<Body>> {
    let encryption = existing(call).await?.encryption.ok_or_else(|| {
        Error::new(
            ErrorCode::ServerSideEncryptionConfigurationNotFoundError,
            "the server side encryption configuration was not found",
        )
    })?;
    Ok(xml_response(StatusCode::OK, document(&encryption)))
}

/// `PUT /{bucket}?encryption`.
pub(crate) async fn put(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let encryption = parse(&document)?;
    crate::routes::buckets::resident(call).await?;
    call.state.storage().keys().check(&encryption.sealing)?;
    if !call
        .state
        .storage()
        .buckets()
        .set_encryption(call.bucket()?, Some(&encryption))
        .await?
    {
        return Err(no_such_bucket());
    }
    Ok(empty_response(StatusCode::OK))
}

/// `DELETE /{bucket}?encryption`.
pub(crate) async fn delete(call: &Call<'_>) -> Result<Response<Body>> {
    if !call
        .state
        .storage()
        .buckets()
        .set_encryption(call.bucket()?, None)
        .await?
    {
        return Err(no_such_bucket());
    }
    Ok(empty_response(StatusCode::NO_CONTENT))
}

#[cfg(test)]
#[path = "encryption_tests.rs"]
mod tests;
