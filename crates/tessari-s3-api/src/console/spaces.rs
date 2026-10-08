//! Spaces through the console: listed and created by operators through the same space service the server uses, each
//! creation recorded with the operator's reason.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{Action, Principal, SpaceName};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::spaces::SpaceCreated;

use super::ConsoleState;
use super::access::allow;
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Serialize)]
struct SpaceView {
    name: String,
    created: String,
}

#[derive(Serialize)]
pub(super) struct Spaces {
    spaces: Vec<SpaceView>,
}

pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Spaces>, ConsoleError> {
    allow(&principal, &Action::Operate)?;
    let spaces = state.storage().spaces().list().await?;
    Ok(Json(Spaces {
        spaces: spaces
            .into_iter()
            .map(|space| SpaceView {
                name: space.name.as_str().to_owned(),
                created: space.created.iso8601_millis(),
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
    allow(&principal, &Action::Operate)?;
    let Create {
        name,
        reason: given,
    } = json(&headers, &body, "expected name and reason")?;
    let why = reason(given, true)?;
    let name = SpaceName::new(&name).ok_or(ConsoleError::invalid(
        "invalid_space_name",
        "space names are 1-63 lowercase letters, digits and inner hyphens",
    ))?;
    let created = state.storage().spaces().create(&name).await?;
    let outcome = match created {
        SpaceCreated::Created(_) => "done",
        SpaceCreated::Exists => "exists",
    };
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "create_space".to_owned(),
            target: name.as_str().to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await?;
    match created {
        SpaceCreated::Created(_) => Ok((
            StatusCode::CREATED,
            Json(CreatedView {
                name: name.as_str().to_owned(),
            }),
        )
            .into_response()),
        SpaceCreated::Exists => Err(ConsoleError::conflict(
            "space_exists",
            "a space of that name already exists",
        )),
    }
}
