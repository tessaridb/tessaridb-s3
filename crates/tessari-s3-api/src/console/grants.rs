//! A user's per-bucket grants through the console. Granting is judged on the user being managed AND on the result: the
//! bucket must belong to that user's space, so a grant never reaches across spaces; a bucket elsewhere answers as a
//! missing one. Every change is recorded with its reason.

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tessari_s3_core::authz::{Access, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;

use super::ConsoleState;
use super::actions::record;
use super::buckets::bucket_name;
use super::error::ConsoleError;
use super::input::{json, reason};
use super::users::managed;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Grant {
    read: bool,
    write: bool,
    reason: Option<String>,
}

pub(super) async fn set(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path((name, bucket)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Grant {
        read,
        write,
        reason: given,
    } = json(&headers, &body, "expected read, write and reason")?;
    let why = reason(given, true)?;
    let user = managed(&state, &principal, &name).await?;
    let bucket = bucket_name(&bucket)?;
    let in_space = state
        .storage()
        .buckets()
        .get(&bucket)
        .await?
        .is_some_and(|found| found.space == user.space);
    if !in_space {
        return Err(ConsoleError::invalid(
            "bucket_not_in_space",
            "there is no such bucket in the user's space",
        ));
    }
    let granted = state
        .storage()
        .users()
        .set_grant(&user.name, &bucket, Access { read, write })
        .await?;
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "set_grant".to_owned(),
            target: format!("{}/{}", user.name.as_str(), bucket.as_str()),
            reason: why,
            outcome: if granted { "done" } else { "no_such_user" }.to_owned(),
        },
    )
    .await?;
    if granted {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ConsoleError::missing(
            "no_such_user",
            "there is no such user",
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Remove {
    reason: Option<String>,
}

pub(super) async fn remove(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path((name, bucket)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Remove { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let user = managed(&state, &principal, &name).await?;
    let bucket = bucket_name(&bucket)?;
    let removed = state
        .storage()
        .users()
        .remove_grant(&user.name, &bucket)
        .await?;
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "remove_grant".to_owned(),
            target: format!("{}/{}", user.name.as_str(), bucket.as_str()),
            reason: why,
            outcome: if removed { "done" } else { "no_such_grant" }.to_owned(),
        },
    )
    .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ConsoleError::missing(
            "no_such_grant",
            "the user has no grant on that bucket",
        ))
    }
}
