//! The action record: every change and every download made through the console is recorded as it ended, with the
//! signed-in key id and the operator's reason; `GET /api/v1/actions` reads it newest first, a page at a time.

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Extension, Query, State};
use serde::{Deserialize, Serialize};
use tessari_s3_constants::CONSOLE_ACTIONS_PAGE_MAX;
use tessari_s3_core::authz::{self, Principal};
use tessari_s3_storage::actions::{Action, NewAction};

use super::ConsoleState;
use super::access::allow;
use super::error::ConsoleError;

/// Records `action`. A record that cannot be written is logged in full and answered as an error, so the operator is
/// told the action is not in the record rather than shown a success.
pub(super) async fn record(state: &ConsoleState, action: NewAction) -> Result<(), ConsoleError> {
    if let Err(error) = state.storage().actions().record(&action).await {
        tracing::error!(
            error = %error,
            operator = %action.operator,
            operation = %action.operation,
            target = %action.target,
            reason = ?action.reason,
            outcome = %action.outcome,
            "console action not recorded"
        );
        return Err(ConsoleError::not_recorded());
    }
    Ok(())
}

#[derive(Deserialize)]
pub(super) struct PageQuery {
    before: Option<u64>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct ActionView {
    position: u64,
    at: String,
    operator: String,
    operation: String,
    target: String,
    reason: Option<String>,
    outcome: String,
}

impl From<Action> for ActionView {
    fn from(action: Action) -> Self {
        Self {
            position: action.position,
            at: action.at.iso8601_millis(),
            operator: action.operator,
            operation: action.operation,
            target: action.target,
            reason: action.reason,
            outcome: action.outcome,
        }
    }
}

#[derive(Serialize)]
pub(super) struct Actions {
    actions: Vec<ActionView>,
    /// The `before` of the next, older page; absent on the oldest.
    next: Option<u64>,
}

pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<Actions>, ConsoleError> {
    // The record spans every space, so only those who operate the store read it.
    allow(&principal, &authz::Action::Operate)?;
    let Query(page) =
        query.map_err(|_| ConsoleError::bad_request("before and limit are whole numbers"))?;
    let limit = page
        .limit
        .unwrap_or(CONSOLE_ACTIONS_PAGE_MAX)
        .clamp(1, CONSOLE_ACTIONS_PAGE_MAX);
    let actions = state.storage().actions().recent(page.before, limit).await?;
    let next = actions
        .last()
        .map(|oldest| oldest.position)
        .filter(|position| actions.len() == limit && *position > 1);
    Ok(Json(Actions {
        actions: actions.into_iter().map(ActionView::from).collect(),
        next,
    }))
}
