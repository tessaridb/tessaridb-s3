//! A group's members and grants through the console. A member must be a user of the group's space and a granted
//! bucket must belong to it — anything elsewhere answers as missing, so a group never reaches across spaces. Every
//! change is recorded with its reason; a member receives a change within the principal window (5 s).

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tessari_s3_core::authz::{Access, Principal, UserName};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::groups::MemberChange;

use super::ConsoleState;
use super::actions::record;
use super::buckets::bucket_name;
use super::error::ConsoleError;
use super::groups::{administered, group_name};
use super::input::{json, reason};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reason {
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Grant {
    read: bool,
    write: bool,
    reason: Option<String>,
}

/// The answer to a change: done, or the refusal its outcome names.
fn answer(change: MemberChange, elsewhere: ConsoleError) -> Result<Response, ConsoleError> {
    match change {
        MemberChange::Done => Ok(StatusCode::NO_CONTENT.into_response()),
        MemberChange::NoSuchGroup => Err(ConsoleError::missing(
            "no_such_group",
            "there is no such group",
        )),
        MemberChange::OtherSpace => Err(elsewhere),
    }
}

const fn outcome(change: MemberChange) -> &'static str {
    match change {
        MemberChange::Done => "done",
        MemberChange::NoSuchGroup => "no_such_group",
        MemberChange::OtherSpace => "other_space",
    }
}

fn no_member() -> ConsoleError {
    ConsoleError::missing("no_such_user", "there is no such user in the group's space")
}

fn no_bucket() -> ConsoleError {
    ConsoleError::invalid(
        "bucket_not_in_space",
        "there is no such bucket in the group's space",
    )
}

pub(super) async fn add_member(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path((space, group, user)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let Reason { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let space = administered(&principal, &space)?;
    let name = group_name(&group)?;
    let user = UserName::new(&user).ok_or_else(no_member)?;
    let found = state
        .storage()
        .users()
        .get(&user)
        .await?
        .ok_or_else(no_member)?;
    let change = state
        .storage()
        .groups()
        .add_member(&space, &name, &user, &found.space)
        .await?;
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "add_group_member".to_owned(),
            target: format!("{}/{}/{}", space.as_str(), name.as_str(), user.as_str()),
            reason: why,
            outcome: outcome(change).to_owned(),
        },
    )
    .await?;
    answer(change, no_member())
}

pub(super) async fn remove_member(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path((space, group, user)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let Reason { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let space = administered(&principal, &space)?;
    let name = group_name(&group)?;
    let user = UserName::new(&user).ok_or_else(no_member)?;
    let removed = state
        .storage()
        .groups()
        .remove_member(&space, &name, &user)
        .await?;
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "remove_group_member".to_owned(),
            target: format!("{}/{}/{}", space.as_str(), name.as_str(), user.as_str()),
            reason: why,
            outcome: if removed { "done" } else { "no_such_member" }.to_owned(),
        },
    )
    .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ConsoleError::missing(
            "no_such_member",
            "the user is not a member of the group",
        ))
    }
}

pub(super) async fn set_grant(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path((space, group, bucket)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let Grant {
        read,
        write,
        reason: given,
    } = json(&headers, &body, "expected read, write and reason")?;
    let why = reason(given, true)?;
    let space = administered(&principal, &space)?;
    let name = group_name(&group)?;
    let bucket = bucket_name(&bucket)?;
    let found = state
        .storage()
        .buckets()
        .get(&bucket)
        .await?
        .ok_or_else(no_bucket)?;
    let change = state
        .storage()
        .groups()
        .set_grant(&space, &name, &bucket, &found.space, Access { read, write })
        .await?;
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "set_group_grant".to_owned(),
            target: format!("{}/{}/{}", space.as_str(), name.as_str(), bucket.as_str()),
            reason: why,
            outcome: outcome(change).to_owned(),
        },
    )
    .await?;
    answer(change, no_bucket())
}

pub(super) async fn remove_grant(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path((space, group, bucket)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let Reason { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let space = administered(&principal, &space)?;
    let name = group_name(&group)?;
    let bucket = bucket_name(&bucket)?;
    let removed = state
        .storage()
        .groups()
        .remove_grant(&space, &name, &bucket)
        .await?;
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "remove_group_grant".to_owned(),
            target: format!("{}/{}/{}", space.as_str(), name.as_str(), bucket.as_str()),
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
            "the group has no grant on that bucket",
        ))
    }
}
