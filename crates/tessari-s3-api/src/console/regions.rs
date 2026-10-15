//! Region failover: `GET /api/v1/regions` and `POST …/failover`, `…/restore`. Failing a region over records the
//! region its writes go to under the next epoch, and the region refuses client writes until it is restored, which
//! takes the next epoch again. Reading needs the cluster view; changing anything is root's, needs a fresh sign-in and a
//! reason, and every attempt is recorded with how it ended.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_infrastructure::tessaridb::is_region_name;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::regions::{FailOver, RegionState, Restore};

use super::ConsoleState;
use super::access::{allow, recently_authenticated};
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Serialize)]
pub(super) struct RegionView {
    region: String,
    epoch: u64,
    failed_over: bool,
    promoted: Option<String>,
    reason: String,
    by: String,
    at: String,
}

impl From<&RegionState> for RegionView {
    fn from(state: &RegionState) -> Self {
        Self {
            region: state.region.clone(),
            epoch: state.epoch,
            failed_over: state.failed_over(),
            promoted: state.promoted.clone(),
            reason: state.reason.clone(),
            by: state.by.clone(),
            at: state.at.iso8601_millis(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct RegionsView {
    regions: Vec<RegionView>,
}

#[derive(Deserialize)]
pub(super) struct FailOverRequest {
    region: String,
    promoted: String,
    reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RestoreRequest {
    region: String,
    reason: Option<String>,
}

fn region(name: &str) -> Result<String, ConsoleError> {
    let name = name.trim();
    if is_region_name(name) {
        Ok(name.to_owned())
    } else {
        Err(ConsoleError::invalid(
            "invalid_region",
            "a region is lowercase letters, digits and single hyphens, beginning with a letter",
        ))
    }
}

fn changed() -> ConsoleError {
    ConsoleError::conflict(
        "region_changed",
        "the region changed while this was decided; read it again",
    )
}

async fn recorded(
    state: &ConsoleState,
    session: &Session,
    operation: &str,
    target: &str,
    why: Option<String>,
    outcome: &str,
) -> Result<(), ConsoleError> {
    record(
        state,
        NewAction {
            operator: session.subject.operator(),
            operation: operation.to_owned(),
            target: target.to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await
}

/// `Ok` while this node's region takes writes; a failed-over region refuses them until it is restored.
pub(super) async fn writable(state: &ConsoleState) -> Result<(), ConsoleError> {
    match state
        .storage()
        .regions()
        .failed_over(state.region())
        .await?
    {
        Some(_) => Err(ConsoleError::failed_over()),
        None => Ok(()),
    }
}

pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<RegionsView>, ConsoleError> {
    allow(&principal, &Action::ViewCluster)?;
    let regions = state.storage().regions().list().await?;
    Ok(Json(RegionsView {
        regions: regions.iter().map(RegionView::from).collect(),
    }))
}

pub(super) async fn fail_over(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<RegionView>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let FailOverRequest {
        region: name,
        promoted,
        reason: given,
    } = json(&headers, &body, "expected region, promoted and reason")?;
    let why = reason(given, true)?;
    let (name, promoted) = (region(&name)?, region(&promoted)?);
    let by = session.subject.operator();
    let answer = state
        .storage()
        .regions()
        .fail_over(&name, &promoted, &by, why.as_deref().unwrap_or_default())
        .await?;
    let (outcome, answer) = match answer {
        FailOver::FailedOver(region) => ("failed_over", Ok(Json(RegionView::from(&region)))),
        FailOver::AlreadyFailedOver(_) => (
            "region_failed_over",
            Err(ConsoleError::conflict(
                "region_failed_over",
                "the region is already failed over; restore it first",
            )),
        ),
        FailOver::ToItself => (
            "invalid_promotion",
            Err(ConsoleError::invalid(
                "invalid_promotion",
                "a region is failed over to another region",
            )),
        ),
        FailOver::Contended => ("region_changed", Err(changed())),
    };
    recorded(&state, &session, "fail_over_region", &name, why, outcome).await?;
    answer
}

pub(super) async fn restore(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<RegionView>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let RestoreRequest {
        region: name,
        reason: given,
    } = json(&headers, &body, "expected region and reason")?;
    let why = reason(given, true)?;
    let name = region(&name)?;
    let by = session.subject.operator();
    let answer = state
        .storage()
        .regions()
        .restore(&name, &by, why.as_deref().unwrap_or_default())
        .await?;
    let (outcome, answer) = match answer {
        Restore::Restored(region) => ("restored", Ok(Json(RegionView::from(&region)))),
        Restore::NotFailedOver => (
            "region_not_failed_over",
            Err(ConsoleError::conflict(
                "region_not_failed_over",
                "the region is not failed over",
            )),
        ),
        Restore::Contended => ("region_changed", Err(changed())),
    };
    recorded(&state, &session, "restore_region", &name, why, outcome).await?;
    answer
}
