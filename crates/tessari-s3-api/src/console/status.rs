//! `GET /api/v1/status`: what this node is — its name in the cluster, the build, the region it signs for — and, on a
//! cluster member, the registered members and how many objects wait for healing.

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use tessari_s3_constants::CONSOLE_BACKLOG_COUNT_MAX;

use super::ConsoleState;
use super::error::ConsoleError;

#[derive(Serialize)]
struct MemberView {
    node: String,
    endpoint: String,
}

#[derive(Serialize)]
struct BacklogView {
    /// Objects listed for healing, counted up to the bound.
    listed: usize,
    /// Whether more are listed than were counted.
    more: bool,
}

#[derive(Serialize)]
pub(super) struct Status {
    version: &'static str,
    node: Option<String>,
    region: String,
    erasure: Option<String>,
    members: Option<Vec<MemberView>>,
    heal_backlog: Option<BacklogView>,
}

pub(super) async fn status(
    State(state): State<ConsoleState>,
) -> Result<Json<Status>, ConsoleError> {
    let storage = state.storage();
    let members = match state.node() {
        Some(_) => Some(
            storage
                .cluster()
                .members()
                .await?
                .into_iter()
                .map(|member| MemberView {
                    node: member.node.as_str().to_owned(),
                    endpoint: member.endpoint,
                })
                .collect(),
        ),
        None => None,
    };
    let heal_backlog = storage
        .objects()
        .heal_backlog(CONSOLE_BACKLOG_COUNT_MAX)
        .await?
        .map(|backlog| BacklogView {
            listed: backlog.listed,
            more: backlog.more,
        });
    Ok(Json(Status {
        version: env!("CARGO_PKG_VERSION"),
        node: state.node().map(str::to_owned),
        region: state.region().to_owned(),
        erasure: state.erasure().map(str::to_owned),
        members,
        heal_backlog,
    }))
}
