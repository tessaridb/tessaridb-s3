//! Tags (S3 ref 05 M8): the `x-amz-tagging` header a write carries, and — below — the object and bucket tagging
//! operations. Every limit is `tessari_s3_core::tagging`'s; a set breaking one is `InvalidTag`.

use std::collections::BTreeMap;

use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode};
use tessari_s3_core::tagging::{BUCKET_TAGS, OBJECT_TAGS, TagError, TagSet};
use tessari_s3_storage::objects::TagsOutcome;
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::buckets::{existing, no_such_bucket, resident};
use super::object_lock::checked_document;
use super::xml_tree::{malformed, tree};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, with_version, xml_response};
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

/// The deepest element a `Tagging` document has: tagging › tag set › tag › key.
const MAX_DEPTH: usize = 4;

/// `InvalidTag`, in the limit's own words.
pub(crate) fn invalid_tag(error: TagError) -> Error {
    Error::new(ErrorCode::InvalidTag, error.to_string())
}

/// The tags an `x-amz-tagging` header names; none without one.
///
/// # Errors
/// `InvalidTag` for a header that is not percent-encoded text or a set S3 refuses.
pub(crate) fn header_tags(headers: &HeaderMap) -> Result<BTreeMap<String, String>> {
    let Some(value) = headers.get("x-amz-tagging") else {
        return Ok(BTreeMap::new());
    };
    let text = value
        .to_str()
        .map_err(|_| invalid_tag(TagError::Encoding))?;
    let set = TagSet::from_header(text, OBJECT_TAGS).map_err(invalid_tag)?;
    Ok(set
        .tags()
        .iter()
        .map(|tag| (tag.key.clone(), tag.value.clone()))
        .collect())
}

/// The tags a `Tagging` document lists, at most `max` of them.
///
/// # Errors
/// `MalformedXML` for a document of another shape; `InvalidTag` for a set S3 refuses.
fn parse(document: &[u8], max: usize) -> Result<BTreeMap<String, String>> {
    let root = tree(document, MAX_DEPTH)?;
    if root.name != "Tagging" {
        return Err(malformed());
    }
    let mut pairs = Vec::new();
    for set in &root.children {
        if set.name != "TagSet" {
            return Err(malformed());
        }
        for tag in &set.children {
            if tag.name != "Tag" {
                return Err(malformed());
            }
            let (mut key, mut value) = (None, None);
            for field in &tag.children {
                let slot = match field.name.as_str() {
                    "Key" => &mut key,
                    "Value" => &mut value,
                    _ => return Err(malformed()),
                };
                if slot.replace(field.text.clone()).is_some() {
                    return Err(malformed());
                }
            }
            pairs.push((key.ok_or_else(malformed)?, value.unwrap_or_default()));
        }
    }
    let set = TagSet::new(pairs, max).map_err(invalid_tag)?;
    Ok(set
        .tags()
        .iter()
        .map(|tag| (tag.key.clone(), tag.value.clone()))
        .collect())
}

/// The `Tagging` document for `tags`.
fn document(tags: &BTreeMap<String, String>) -> String {
    let listed: String = tags
        .iter()
        .map(|(key, value)| {
            format!(
                "<Tag><Key>{}</Key><Value>{}</Value></Tag>",
                escape(key),
                escape(value)
            )
        })
        .collect();
    format!("<Tagging xmlns=\"{S3_NAMESPACE}\"><TagSet>{listed}</TagSet></Tagging>")
}

/// The version a tagging request found, or the error S3 answers when there is none.
fn found(outcome: TagsOutcome) -> Result<(String, BTreeMap<String, String>)> {
    match outcome {
        TagsOutcome::Version { version_id, tags } => Ok((version_id, tags)),
        TagsOutcome::NoSuchBucket => Err(no_such_bucket()),
        TagsOutcome::NoSuchKey => Err(Error::new(
            ErrorCode::NoSuchKey,
            "the specified key does not exist",
        )),
        TagsOutcome::NoSuchVersion => Err(Error::new(
            ErrorCode::NoSuchVersion,
            "the specified version does not exist",
        )),
        TagsOutcome::Malformed => Err(Error::new(
            ErrorCode::InvalidArgument,
            "Invalid version id specified",
        )),
        TagsOutcome::Marker => Err(Error::new(
            ErrorCode::MethodNotAllowed,
            "the specified method is not allowed against a delete marker",
        )),
    }
}

/// The version id an answer names: none while the bucket was never versioned, as a read of its objects answers.
async fn named_version(call: &Call<'_>, version_id: String) -> Result<Option<String>> {
    Ok(existing(call).await?.versioning.map(|_| version_id))
}

/// `GET /{bucket}/{key}?tagging`: the tags of the version `versionId` names, or of the current one.
pub(crate) async fn get_object(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    let outcome = call
        .state
        .storage()
        .objects()
        .version_tags(call.bucket()?, key, call.query_value("versionId"))
        .await?;
    let (version_id, tags) = found(outcome)?;
    let named = named_version(call, version_id).await?;
    Ok(with_version(
        xml_response(StatusCode::OK, document(&tags)),
        named.as_deref(),
    ))
}

/// Replaces the tags of the version `versionId` names, or of the current one.
async fn set_object(
    call: &Call<'_>,
    key: &ObjectKey,
    tags: &BTreeMap<String, String>,
) -> Result<Option<String>> {
    let outcome = call
        .state
        .storage()
        .objects()
        .set_version_tags(call.bucket()?, key, call.query_value("versionId"), tags)
        .await?;
    let (version_id, _) = found(outcome)?;
    named_version(call, version_id).await
}

/// `PUT /{bucket}/{key}?tagging`.
pub(crate) async fn put_object(
    call: &Call<'_>,
    key: &ObjectKey,
    body: Body,
) -> Result<Response<Body>> {
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let tags = parse(&document, OBJECT_TAGS)?;
    let version_id = set_object(call, key, &tags).await?;
    Ok(with_version(
        empty_response(StatusCode::OK),
        version_id.as_deref(),
    ))
}

/// `DELETE /{bucket}/{key}?tagging`.
pub(crate) async fn delete_object(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    let version_id = set_object(call, key, &BTreeMap::new()).await?;
    Ok(with_version(
        empty_response(StatusCode::NO_CONTENT),
        version_id.as_deref(),
    ))
}

/// `GET /{bucket}?tagging`; a bucket with no tags answers `NoSuchTagSet`.
pub(crate) async fn get_bucket(call: &Call<'_>) -> Result<Response<Body>> {
    let bucket = existing(call).await?;
    if bucket.tags.is_empty() {
        return Err(Error::new(
            ErrorCode::NoSuchTagSet,
            "The TagSet does not exist",
        ));
    }
    Ok(xml_response(StatusCode::OK, document(&bucket.tags)))
}

/// Replaces the bucket's tags; none removes them.
async fn set_bucket(call: &Call<'_>, tags: &BTreeMap<String, String>) -> Result<Response<Body>> {
    if call
        .state
        .storage()
        .buckets()
        .set_tags(call.bucket()?, tags)
        .await?
    {
        Ok(empty_response(StatusCode::NO_CONTENT))
    } else {
        Err(no_such_bucket())
    }
}

/// `PUT /{bucket}?tagging`, which S3 takes only with its `Content-MD5` or a checksum.
pub(crate) async fn put_bucket(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let document = checked_document(call, body).await?;
    let tags = parse(&document, BUCKET_TAGS)?;
    resident(call).await?;
    set_bucket(call, &tags).await
}

/// `DELETE /{bucket}?tagging`.
pub(crate) async fn delete_bucket(call: &Call<'_>) -> Result<Response<Body>> {
    set_bucket(call, &BTreeMap::new()).await
}

#[cfg(test)]
#[path = "tagging_tests.rs"]
mod tests;
