//! Cluster membership: `GET /api/v1/cluster/members` and `POST …/members/approve`, `…/token`, `…/remove`. A node is
//! admitted by its id, pinned to one certificate's fingerprint, or by a one-time join token shown in this answer and
//! never again; a removed node is a tombstone, and a node the current layout places shards on is not removed. Reading
//! needs the cluster view; changing anything is root's, needs a fresh sign-in and a reason, and every attempt is
//! recorded with how it ended — never with the token.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_core::erasure::Code;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::members::{
    Approval, Fingerprint, Issued, MemberState, Membership, Removal, TokenLifetime,
};
use tessari_s3_types::NodeId;

use super::ConsoleState;
use super::access::{allow, recently_authenticated};
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Serialize)]
pub(super) struct MemberView {
    node: String,
    /// `approved` or `removed`.
    state: &'static str,
    fingerprint: Option<String>,
    /// A join token was issued and not yet used.
    awaiting_token: bool,
    token_expires: Option<String>,
    reason: String,
    by: String,
    at: String,
}

impl From<&Membership> for MemberView {
    fn from(member: &Membership) -> Self {
        Self {
            node: member.node.as_str().to_owned(),
            state: match member.state {
                MemberState::Approved => "approved",
                MemberState::Removed => "removed",
            },
            fingerprint: member.fingerprint.as_ref().map(|f| f.as_str().to_owned()),
            awaiting_token: member.token_expires.is_some(),
            token_expires: member.token_expires.map(|at| at.iso8601_millis()),
            reason: member.reason.clone(),
            by: member.by.clone(),
            at: member.at.iso8601_millis(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct MembersView {
    members: Vec<MemberView>,
}

#[derive(Serialize)]
pub(super) struct TokenView {
    node: String,
    /// The join token, shown this once.
    token: String,
    expires: String,
}

#[derive(Deserialize)]
pub(super) struct ApproveRequest {
    node: String,
    fingerprint: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct TokenRequest {
    node: String,
    lifetime_secs: u64,
    reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RemoveRequest {
    node: String,
    reason: Option<String>,
}

/// The node's erasure code; membership belongs to a cluster.
fn code(state: &ConsoleState) -> Result<Code, ConsoleError> {
    state.code().ok_or(ConsoleError::conflict(
        "not_a_cluster",
        "this node is not a cluster member, so it has no members",
    ))
}

fn node(id: &str) -> Result<NodeId, ConsoleError> {
    NodeId::new(id.trim()).map_err(|_| {
        ConsoleError::invalid(
            "invalid_node_id",
            "a node id is 1 to 63 lowercase letters, digits and inner hyphens",
        )
    })
}

fn removed() -> ConsoleError {
    ConsoleError::conflict(
        "member_removed",
        "this node was removed; it is never admitted again",
    )
}

async fn recorded(
    state: &ConsoleState,
    session: &Session,
    operation: &str,
    node: &NodeId,
    why: Option<String>,
    outcome: &str,
) -> Result<(), ConsoleError> {
    record(
        state,
        NewAction {
            operator: session.subject.operator(),
            operation: operation.to_owned(),
            target: node.as_str().to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await
}

pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<MembersView>, ConsoleError> {
    allow(&principal, &Action::ViewCluster)?;
    code(&state)?;
    let members = state.storage().members().list().await?;
    Ok(Json(MembersView {
        members: members.iter().map(MemberView::from).collect(),
    }))
}

pub(super) async fn approve(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<MemberView>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let ApproveRequest {
        node: id,
        fingerprint,
        reason: given,
    } = json(&headers, &body, "expected node, fingerprint and reason")?;
    let why = reason(given, true)?;
    let node = node(&id)?;
    let fingerprint = fingerprint
        .map(|text| {
            Fingerprint::new(text.trim()).ok_or(ConsoleError::invalid(
                "invalid_fingerprint",
                "a fingerprint is a certificate's SHA-256 as 64 lowercase hex digits",
            ))
        })
        .transpose()?;
    code(&state)?;
    let by = session.subject.operator();
    let approved = state
        .storage()
        .members()
        .approve(
            &node,
            fingerprint.as_ref(),
            &by,
            why.as_deref().unwrap_or_default(),
        )
        .await?;
    let (outcome, answer) = match approved {
        Approval::Approved(member) => ("approved", Ok(Json(MemberView::from(&member)))),
        Approval::Removed => ("member_removed", Err(removed())),
    };
    recorded(&state, &session, "approve_member", &node, why, outcome).await?;
    answer
}

pub(super) async fn token(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let TokenRequest {
        node: id,
        lifetime_secs,
        reason: given,
    } = json(&headers, &body, "expected node, lifetime_secs and reason")?;
    let why = reason(given, true)?;
    let node = node(&id)?;
    let lifetime = TokenLifetime::new(lifetime_secs).ok_or(ConsoleError::invalid(
        "invalid_lifetime",
        "a join token lives from 60 seconds to one day",
    ))?;
    code(&state)?;
    let by = session.subject.operator();
    let issued = state
        .storage()
        .members()
        .issue_token(&node, lifetime, &by, why.as_deref().unwrap_or_default())
        .await?;
    let (outcome, answer) = match issued {
        Issued::Issued { token, expires } => {
            let view = TokenView {
                node: node.as_str().to_owned(),
                token: token.expose().to_owned(),
                expires: expires.iso8601_millis(),
            };
            let mut response = Json(view).into_response();
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            ("issued", Ok(response))
        }
        Issued::Removed => ("member_removed", Err(removed())),
    };
    recorded(&state, &session, "issue_join_token", &node, why, outcome).await?;
    answer
}

pub(super) async fn remove(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let RemoveRequest {
        node: id,
        reason: given,
    } = json(&headers, &body, "expected node and reason")?;
    let why = reason(given, true)?;
    let node = node(&id)?;
    let code = code(&state)?;
    let current = state.storage().cluster().current(code).await?;
    let by = session.subject.operator();
    let removal = state
        .storage()
        .members()
        .remove(
            &node,
            &by,
            why.as_deref().unwrap_or_default(),
            current.as_ref(),
        )
        .await?;
    let (outcome, answer) = match removal {
        Removal::Removed => ("removed", Ok(StatusCode::NO_CONTENT.into_response())),
        Removal::AlreadyRemoved => ("member_removed", Err(removed())),
        Removal::Unknown => (
            "no_such_member",
            Err(ConsoleError::missing(
                "no_such_member",
                "no such cluster member",
            )),
        ),
        Removal::InLayout => (
            "member_in_layout",
            Err(ConsoleError::conflict(
                "member_in_layout",
                "the current layout places shards on this node; apply a layout without it first",
            )),
        ),
    };
    recorded(&state, &session, "remove_member", &node, why, outcome).await?;
    answer
}
