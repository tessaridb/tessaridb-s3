//! Buckets through the console: listed, created and deleted by the same bucket service the S3 handlers call, each
//! change recorded with the operator's reason — required to delete.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::buckets::{Created, Deleted};
use tessari_s3_types::BucketName;

use super::ConsoleState;
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Serialize)]
struct BucketView {
    name: String,
    created: String,
    region: String,
}

#[derive(Serialize)]
pub(super) struct Buckets {
    buckets: Vec<BucketView>,
}

/// The bucket called `name`, or the refusal for a name S3 would refuse.
pub(super) fn bucket_name(name: &str) -> Result<BucketName, ConsoleError> {
    BucketName::new(name).map_err(|_| {
        ConsoleError::invalid(
            "invalid_bucket_name",
            "bucket names are 3-63 lowercase letters, digits, dots and hyphens",
        )
    })
}

pub(super) async fn list(State(state): State<ConsoleState>) -> Result<Json<Buckets>, ConsoleError> {
    let buckets = state.storage().buckets().list().await?;
    Ok(Json(Buckets {
        buckets: buckets
            .into_iter()
            .map(|bucket| BucketView {
                name: bucket.name.as_str().to_owned(),
                created: bucket.created.iso8601_millis(),
                region: bucket.region,
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
pub(super) struct Create {
    name: String,
    reason: Option<String>,
}

#[derive(Serialize)]
struct CreatedView {
    name: String,
}

pub(super) async fn create(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Create {
        name,
        reason: given,
    } = json(&headers, &body, "expected name and reason")?;
    let why = reason(given, false)?;
    let name = bucket_name(&name)?;
    let created = state
        .storage()
        .buckets()
        .create(&name, state.region())
        .await?;
    let outcome = match created {
        Created::Created(_) => "done",
        Created::AlreadyOwned => "exists",
    };
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "create_bucket".to_owned(),
            target: name.as_str().to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await?;
    match created {
        Created::Created(_) => Ok((
            StatusCode::CREATED,
            Json(CreatedView {
                name: name.as_str().to_owned(),
            }),
        )
            .into_response()),
        Created::AlreadyOwned => Err(ConsoleError::conflict(
            "bucket_exists",
            "a bucket of that name already exists",
        )),
    }
}

#[derive(Deserialize)]
pub(super) struct Delete {
    reason: Option<String>,
}

pub(super) async fn delete(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Path(bucket): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Delete { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let name = bucket_name(&bucket)?;
    let deleted = state.storage().buckets().delete(&name).await?;
    let outcome = match deleted {
        Deleted::Deleted => "done",
        Deleted::NoSuchBucket => "no_such_bucket",
        Deleted::NotEmpty => "not_empty",
    };
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "delete_bucket".to_owned(),
            target: name.as_str().to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await?;
    match deleted {
        Deleted::Deleted => Ok(StatusCode::NO_CONTENT.into_response()),
        Deleted::NoSuchBucket => Err(ConsoleError::missing(
            "no_such_bucket",
            "there is no such bucket",
        )),
        Deleted::NotEmpty => Err(ConsoleError::conflict(
            "bucket_not_empty",
            "the bucket still holds objects; delete them first",
        )),
    }
}
