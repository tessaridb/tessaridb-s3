//! Buckets through the console: listed, created and deleted by the same bucket service the S3 handlers call, each
//! change recorded with the operator's reason — required to delete.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{Action, Principal, SpaceName};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::buckets::{Acknowledgement, Created, Deleted};
use tessari_s3_storage::encryption::Sealing;
use tessari_s3_types::BucketName;

use super::ConsoleState;
use super::access::{allow, on_bucket};
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Serialize)]
struct BucketView {
    name: String,
    created: String,
    region: String,
    max_bytes: Option<u64>,
    max_objects: Option<u64>,
    /// `region` when its writes are acknowledged by its region's majority alone, `majority` otherwise.
    acknowledgement: &'static str,
    /// The default encryption of writes that ask for none; null while there is none.
    encryption: Option<EncryptionView>,
}

#[derive(Serialize)]
struct EncryptionView {
    /// `AES256` or `aws:kms`.
    algorithm: &'static str,
    /// The named key under `aws:kms`.
    key: Option<String>,
    /// Whether writes may bring their own key (SSE-C).
    customer_keys: bool,
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

pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Buckets>, ConsoleError> {
    let buckets = state.storage().buckets().list(&principal.visible()).await?;
    Ok(Json(Buckets {
        buckets: buckets
            .into_iter()
            .map(|bucket| BucketView {
                name: bucket.name.as_str().to_owned(),
                created: bucket.created.iso8601_millis(),
                region: bucket.region,
                max_bytes: bucket.quota.max_bytes,
                max_objects: bucket.quota.max_objects,
                acknowledgement: match bucket.acknowledgement {
                    Acknowledgement::Majority => "majority",
                    Acknowledgement::RegionMajority => "region",
                },
                encryption: bucket
                    .encryption
                    .map(|encryption| match encryption.sealing {
                        Sealing::Server => EncryptionView {
                            algorithm: "AES256",
                            key: None,
                            customer_keys: encryption.allow_customer_keys,
                        },
                        Sealing::Named(name) => EncryptionView {
                            algorithm: "aws:kms",
                            key: Some(name.as_str().to_owned()),
                            customer_keys: encryption.allow_customer_keys,
                        },
                    }),
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
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Create {
        name,
        reason: given,
    } = json(&headers, &body, "expected name and reason")?;
    let why = reason(given, false)?;
    let name = bucket_name(&name)?;
    super::regions::writable(&state).await?;
    let (space, creator) = match &principal {
        Principal::Root => (SpaceName::default_space(), None),
        Principal::User(user) => (user.space.clone(), Some(user.name.as_str())),
        // Refused before it creates anything; named for the match, never for the bucket.
        Principal::Upload(upload) => (upload.parent.space.clone(), None),
    };
    allow(&principal, &Action::CreateBucket(space.clone()))?;
    let created = state
        .storage()
        .buckets()
        .create(&name, state.region(), &space, creator, false)
        .await?;
    let outcome = match created {
        Created::Created(_) => "done",
        Created::Exists => "exists",
    };
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
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
        Created::Exists => Err(ConsoleError::conflict(
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
    Extension(principal): Extension<Principal>,
    Path(bucket): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let Delete { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let name = bucket_name(&bucket)?;
    on_bucket(&state, &principal, &name, Action::DeleteBucket).await?;
    super::regions::writable(&state).await?;
    let deleted = state.storage().buckets().delete(&name).await?;
    let outcome = match deleted {
        Deleted::Deleted => "done",
        Deleted::NoSuchBucket => "no_such_bucket",
        Deleted::NotEmpty => "not_empty",
    };
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
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
