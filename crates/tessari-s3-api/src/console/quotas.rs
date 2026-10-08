//! A bucket's quota through the console: set by operators only (ADR-0007), through the same bucket service the
//! storage layer enforces it from, with a reason that is recorded. The PUT carries both limits, so repeating it
//! changes nothing.

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_core::quota::Quota;
use tessari_s3_storage::actions::NewAction;

use super::ConsoleState;
use super::access::allow;
use super::actions::record;
use super::buckets::bucket_name;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetQuota {
    max_bytes: Option<u64>,
    max_objects: Option<u64>,
    reason: Option<String>,
}

/// A limit the metadata store can hold — it keeps them as signed 64-bit integers.
fn storable(limit: Option<u64>) -> Result<Option<u64>, ConsoleError> {
    match limit {
        Some(value) if i64::try_from(value).is_err() => Err(ConsoleError::invalid(
            "invalid_quota",
            "a limit is at most 9223372036854775807",
        )),
        _ => Ok(limit),
    }
}

pub(super) async fn set(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(bucket): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let SetQuota {
        max_bytes,
        max_objects,
        reason: given,
    } = json(
        &headers,
        &body,
        "expected max_bytes, max_objects and reason",
    )?;
    let why = reason(given, true)?;
    allow(&principal, &Action::Operate)?;
    let name = bucket_name(&bucket)?;
    let quota = Quota {
        max_bytes: storable(max_bytes)?,
        max_objects: storable(max_objects)?,
    };
    let set = state.storage().buckets().set_quota(&name, quota).await?;
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "set_quota".to_owned(),
            target: name.as_str().to_owned(),
            reason: why,
            outcome: if set { "done" } else { "no_such_bucket" }.to_owned(),
        },
    )
    .await?;
    if set {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ConsoleError::missing(
            "no_such_bucket",
            "there is no such bucket",
        ))
    }
}
