//! A space's groups through the console: listed, created, described and removed by the same group service the S3
//! server's principals read. Every route is judged as administering the group's space (its admin or an operator); a
//! space the caller does not administer answers alike whether it exists or not. Every change is recorded with its
//! reason.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{Action, GroupName, Principal, SpaceName};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::groups::Group;

use super::ConsoleState;
use super::access::allow;
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};

/// Space `name`, once `principal` may administer it.
pub(super) fn administered(principal: &Principal, name: &str) -> Result<SpaceName, ConsoleError> {
    let space = SpaceName::new(name).ok_or(ConsoleError::invalid(
        "invalid_space_name",
        "space names are 1-63 lowercase letters, digits and inner hyphens",
    ))?;
    allow(principal, &Action::ManageSpace(space.clone()))?;
    Ok(space)
}

/// The group name `text` spells.
pub(super) fn group_name(text: &str) -> Result<GroupName, ConsoleError> {
    GroupName::new(text).ok_or(ConsoleError::invalid(
        "invalid_group_name",
        "group names are 1-63 lowercase letters, digits and inner hyphens",
    ))
}

#[derive(Serialize)]
struct GroupView {
    name: String,
    created: String,
}

impl From<Group> for GroupView {
    fn from(group: Group) -> Self {
        Self {
            name: group.name.as_str().to_owned(),
            created: group.created.iso8601_millis(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct Groups {
    groups: Vec<GroupView>,
}

pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
) -> Result<Json<Groups>, ConsoleError> {
    let space = administered(&principal, &space)?;
    let groups = state.storage().groups().list(&space).await?;
    Ok(Json(Groups {
        groups: groups.into_iter().map(GroupView::from).collect(),
    }))
}

#[derive(Serialize)]
struct GrantView {
    bucket: String,
    read: bool,
    write: bool,
}

#[derive(Serialize)]
pub(super) struct Detail {
    name: String,
    created: String,
    members: Vec<String>,
    grants: Vec<GrantView>,
}

pub(super) async fn detail(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
    Path((space, group)): Path<(String, String)>,
) -> Result<Json<Detail>, ConsoleError> {
    let space = administered(&principal, &space)?;
    let name = group_name(&group)?;
    let groups = state.storage().groups();
    let found = groups
        .get(&space, &name)
        .await?
        .ok_or(ConsoleError::missing(
            "no_such_group",
            "there is no such group",
        ))?;
    let members = groups.members(&space, &name).await?;
    let grants = groups.grants(&space, &name).await?;
    Ok(Json(Detail {
        name: found.name.as_str().to_owned(),
        created: found.created.iso8601_millis(),
        members,
        grants: grants
            .into_iter()
            .map(|grant| GrantView {
                bucket: grant.bucket,
                read: grant.access.read,
                write: grant.access.write,
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Create {
    name: String,
    reason: Option<String>,
}

pub(super) async fn create(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let Create {
        name,
        reason: given,
    } = json(&headers, &body, "expected name and reason")?;
    let why = reason(given, true)?;
    let space = administered(&principal, &space)?;
    let name = group_name(&name)?;
    if state.storage().spaces().get(&space).await?.is_none() {
        return Err(ConsoleError::missing(
            "no_such_space",
            "there is no such space",
        ));
    }
    let created = state.storage().groups().create(&space, &name).await?;
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "create_group".to_owned(),
            target: format!("{}/{}", space.as_str(), name.as_str()),
            reason: why,
            outcome: if created { "done" } else { "exists" }.to_owned(),
        },
    )
    .await?;
    if created {
        Ok(StatusCode::CREATED.into_response())
    } else {
        Err(ConsoleError::conflict(
            "group_exists",
            "a group of that name already exists in the space",
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Remove {
    reason: Option<String>,
}

pub(super) async fn delete(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path((space, group)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let Remove { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let space = administered(&principal, &space)?;
    let name = group_name(&group)?;
    let removed = state.storage().groups().delete(&space, &name).await?;
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "delete_group".to_owned(),
            target: format!("{}/{}", space.as_str(), name.as_str()),
            reason: why,
            outcome: if removed { "done" } else { "no_such_group" }.to_owned(),
        },
    )
    .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ConsoleError::missing(
            "no_such_group",
            "there is no such group",
        ))
    }
}
