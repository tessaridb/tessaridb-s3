//! Changing the layout objects are placed by: `GET`/`PUT`/`DELETE /api/v1/cluster/layout/staged` and `POST
//! /api/v1/cluster/layout/apply`. Staging is judged against the registered nodes and the layout rules; applying makes
//! the staged layout the next version only while nothing moved since — the apply names the staging it was shown, so a
//! layout staged again meanwhile is not applied unseen. Changing anything is root's, needs a fresh sign-in and a
//! reason, and every attempt is recorded with how it ended.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_core::erasure::{Code, LayoutError};
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::cluster::{Apply, Stage, StagedLayout};
use tessari_s3_types::NodeId;

use super::ConsoleState;
use super::access::{allow, recently_authenticated};
use super::actions::record;
use super::cluster::LayoutView;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Serialize)]
pub(super) struct StagedView {
    /// Names this staging; an apply sends it back.
    id: String,
    /// The version it was staged over.
    base: u32,
    /// The layout it becomes: version `base` + 1.
    layout: LayoutView,
}

impl From<&StagedLayout> for StagedView {
    fn from(staged: &StagedLayout) -> Self {
        Self {
            id: staged.id.clone(),
            base: staged.base,
            layout: LayoutView::from(&staged.layout),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct StageRequest {
    /// The erasure sets, each a list of node ids, one per shard.
    sets: Vec<Vec<String>>,
    reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Discard {
    reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ApplyRequest {
    /// The staging shown to the operator.
    id: String,
    reason: Option<String>,
}

const TARGET: &str = "cluster";

/// The node's erasure code; a node on its own has no layout to change.
fn code(state: &ConsoleState) -> Result<Code, ConsoleError> {
    state.code().ok_or(ConsoleError::conflict(
        "not_a_cluster",
        "this node is not a cluster member, so it has no layout",
    ))
}

/// A broken rule in the console's words.
fn invalid(error: &LayoutError) -> ConsoleError {
    let message = match error {
        LayoutError::Width { .. } => "every set names exactly one node per shard of the code",
        LayoutError::Duplicate => "a node is named twice in the layout",
        LayoutError::ZoneOverloaded { .. } => {
            "a set places more shards in one zone than the code survives losing"
        }
        LayoutError::NoSets | LayoutError::Version => "a layout holds at least one set",
    };
    ConsoleError::invalid("invalid_layout", message)
}

async fn recorded(
    state: &ConsoleState,
    session: &Session,
    operation: &str,
    why: Option<String>,
    outcome: &str,
) -> Result<(), ConsoleError> {
    record(
        state,
        NewAction {
            operator: session.subject.operator(),
            operation: operation.to_owned(),
            target: TARGET.to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await
}

pub(super) async fn staged(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<StagedView>, ConsoleError> {
    allow(&principal, &Action::ViewCluster)?;
    let code = code(&state)?;
    match state.storage().cluster().staged(code).await? {
        Some(staged) => Ok(Json(StagedView::from(&staged))),
        None => Err(nothing_staged()),
    }
}

fn nothing_staged() -> ConsoleError {
    ConsoleError::missing("nothing_staged", "no layout is staged")
}

pub(super) async fn stage(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<StagedView>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let StageRequest {
        sets,
        reason: given,
    } = json(&headers, &body, "expected sets and reason")?;
    let why = reason(given, true)?;
    let code = code(&state)?;
    let sets = sets
        .iter()
        .map(|set| {
            set.iter()
                .map(|id| NodeId::new(id.trim()))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            ConsoleError::invalid(
                "invalid_node_id",
                "a node id is 1 to 63 lowercase letters, digits and inner hyphens",
            )
        })?;
    let (outcome, answer) = match state.storage().cluster().stage(code, sets).await? {
        Stage::Staged(staged) => ("staged", Ok(Json(StagedView::from(&staged)))),
        Stage::NoLayoutYet => (
            "no_layout_yet",
            Err(ConsoleError::conflict(
                "no_layout_yet",
                "there is no layout yet: enough nodes have not registered",
            )),
        ),
        Stage::Unregistered(_) => (
            "unregistered_node",
            Err(ConsoleError::invalid(
                "unregistered_node",
                "a node the layout names has not registered with the cluster",
            )),
        ),
        Stage::Invalid(error) => ("invalid_layout", Err(invalid(&error))),
    };
    recorded(&state, &session, "stage_layout", why, outcome).await?;
    answer
}

pub(super) async fn discard(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let Discard { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    code(&state)?;
    state.storage().cluster().unstage().await?;
    recorded(&state, &session, "discard_layout", why, "done").await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub(super) async fn apply(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<LayoutView>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let ApplyRequest { id, reason: given } = json(&headers, &body, "expected id and reason")?;
    let why = reason(given, true)?;
    let code = code(&state)?;
    let (outcome, answer) = match state.storage().cluster().apply(code, &id).await? {
        Apply::Applied(layout) => ("applied", Ok(Json(LayoutView::from(&layout)))),
        Apply::NothingStaged => ("nothing_staged", Err(nothing_staged())),
        Apply::StagedChanged => (
            "staged_changed",
            Err(ConsoleError::conflict(
                "staged_changed",
                "the staged layout changed since it was shown; review it again",
            )),
        ),
        Apply::BaseMoved => (
            "base_moved",
            Err(ConsoleError::conflict(
                "base_moved",
                "the layout changed since this one was staged; stage it again over the current one",
            )),
        ),
    };
    recorded(&state, &session, "apply_layout", why, outcome).await?;
    answer
}
