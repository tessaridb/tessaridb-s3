//! Objects through the console: a bucket listed a page at a time in S3's byte order (with prefix and delimiter), one
//! object described, and one object deleted only while it is still the version the operator looked at — its ETag is
//! the condition, so a change made in between is refused rather than lost.

use std::collections::BTreeMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_constants::{CONSOLE_OBJECTS_PAGE_DEFAULT, LIST_MAX_KEYS};
use tessari_s3_core::console::Session;
use tessari_s3_core::objects::listing::{Entry, ListSpec, Resume};
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::objects::{Content, Listed, RemovedIf};
use tessari_s3_types::ObjectKey;

use super::ConsoleState;
use super::actions::record;
use super::buckets::bucket_name;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Deserialize)]
pub(super) struct ListQuery {
    prefix: Option<String>,
    delimiter: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct ObjectRow {
    key: String,
    size: u64,
    etag: String,
    modified: String,
}

#[derive(Serialize)]
pub(super) struct Objects {
    objects: Vec<ObjectRow>,
    prefixes: Vec<String>,
    /// Where the next page starts; absent on the last page.
    next: Option<String>,
}

/// A page position as the listing hands it out: `key:<key>` or `prefix:<prefix>`.
fn cursor(resume: &Resume) -> String {
    match resume {
        Resume::Key(key) => format!("key:{key}"),
        Resume::Prefix(prefix) => format!("prefix:{prefix}"),
    }
}

fn resume(cursor: &str) -> Result<Resume, ConsoleError> {
    if let Some(key) = cursor.strip_prefix("key:") {
        return Ok(Resume::Key(key.to_owned()));
    }
    cursor
        .strip_prefix("prefix:")
        .map(|prefix| Resume::Prefix(prefix.to_owned()))
        .ok_or_else(|| ConsoleError::bad_request("cursor is the next value of a previous page"))
}

/// The key a query names, or the refusal.
pub(super) fn object_key(key: Option<&str>) -> Result<ObjectKey, ConsoleError> {
    ObjectKey::new(key.unwrap_or_default())
        .map_err(|_| ConsoleError::invalid("invalid_key", "key is required, at most 1,024 bytes"))
}

pub(super) const fn no_such_bucket() -> ConsoleError {
    ConsoleError::missing("no_such_bucket", "there is no such bucket")
}

pub(super) const fn no_such_key() -> ConsoleError {
    ConsoleError::missing("no_such_key", "there is no such object")
}

pub(super) async fn list(
    State(state): State<ConsoleState>,
    Path(bucket): Path<String>,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<Objects>, ConsoleError> {
    let Query(query) = query.map_err(|_| ConsoleError::bad_request("limit is a whole number"))?;
    let bucket = bucket_name(&bucket)?;
    let spec = ListSpec {
        prefix: query.prefix.unwrap_or_default(),
        delimiter: query.delimiter.filter(|delimiter| !delimiter.is_empty()),
        max_keys: query
            .limit
            .unwrap_or(CONSOLE_OBJECTS_PAGE_DEFAULT)
            .clamp(1, LIST_MAX_KEYS),
        start_after: None,
        resume: query.cursor.as_deref().map(resume).transpose()?,
    };
    let page = match state.storage().objects().list(&bucket, spec).await? {
        Listed::Page(page) => page,
        Listed::NoSuchBucket => return Err(no_such_bucket()),
    };
    let next = page.resume().as_ref().map(cursor);
    let mut objects = Vec::new();
    let mut prefixes = Vec::new();
    for entry in page.entries {
        match entry {
            Entry::Object { key, summary } => objects.push(ObjectRow {
                key,
                size: summary.size,
                etag: summary.etag,
                modified: summary.modified.iso8601_millis(),
            }),
            Entry::Prefix(prefix) => prefixes.push(prefix),
        }
    }
    Ok(Json(Objects {
        objects,
        prefixes,
        next,
    }))
}

#[derive(Deserialize)]
pub(super) struct KeyQuery {
    key: Option<String>,
}

#[derive(Serialize)]
pub(super) struct Detail {
    key: String,
    size: u64,
    etag: String,
    modified: String,
    headers: BTreeMap<String, String>,
    metadata: BTreeMap<String, String>,
    checksums: BTreeMap<String, String>,
    /// How many parts a multipart object was completed from; absent otherwise.
    parts: Option<usize>,
}

pub(super) async fn detail(
    State(state): State<ConsoleState>,
    Path(bucket): Path<String>,
    query: Result<Query<KeyQuery>, QueryRejection>,
) -> Result<Json<Detail>, ConsoleError> {
    let Query(query) = query.map_err(|_| ConsoleError::bad_request("key is required"))?;
    let bucket = bucket_name(&bucket)?;
    let key = object_key(query.key.as_deref())?;
    let object = match state.storage().objects().get(&bucket, &key).await? {
        Err(()) => return Err(no_such_bucket()),
        Ok(None) => return Err(no_such_key()),
        Ok(Some(object)) => object,
    };
    let parts = match &object.content {
        Content::Parts(multipart) => Some(multipart.parts.len()),
        Content::Inline(_) | Content::Data(_) => None,
    };
    Ok(Json(Detail {
        key: key.as_str().to_owned(),
        size: object.size,
        etag: object.etag,
        modified: object.modified.iso8601_millis(),
        headers: object.headers,
        metadata: object.metadata,
        checksums: object.checksums,
        parts,
    }))
}

#[derive(Deserialize)]
pub(super) struct Delete {
    etag: String,
    reason: Option<String>,
}

pub(super) async fn delete(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Path(bucket): Path<String>,
    query: Result<Query<KeyQuery>, QueryRejection>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Query(query) = query.map_err(|_| ConsoleError::bad_request("key is required"))?;
    let Delete {
        etag,
        reason: given,
    } = json(&headers, &body, "expected etag and reason")?;
    let why = reason(given, true)?;
    let bucket = bucket_name(&bucket)?;
    let key = object_key(query.key.as_deref())?;
    let removed = state
        .storage()
        .objects()
        .delete_if_match(&bucket, &key, &etag)
        .await?;
    let outcome = match removed {
        RemovedIf::Done => "done",
        RemovedIf::NoSuchBucket => "no_such_bucket",
        RemovedIf::NoSuchKey => "no_such_key",
        RemovedIf::PreconditionFailed => "precondition_failed",
    };
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "delete_object".to_owned(),
            target: format!("{}/{}", bucket.as_str(), key.as_str()),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await?;
    match removed {
        RemovedIf::Done => Ok(StatusCode::NO_CONTENT.into_response()),
        RemovedIf::NoSuchBucket => Err(no_such_bucket()),
        RemovedIf::NoSuchKey => Err(no_such_key()),
        RemovedIf::PreconditionFailed => Err(ConsoleError::precondition_failed()),
    }
}
