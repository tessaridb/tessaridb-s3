//! `GET /api/v1/status`: what this node is — its name in the cluster, the build, the region it signs for — and, on a
//! cluster member, the registered members with whether each answers now, and how many objects wait for healing.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use futures_util::stream::{self, StreamExt};
use serde::Serialize;
use tessari_s3_constants::{
    CONSOLE_BACKLOG_COUNT_MAX, CONSOLE_MEMBER_PROBE_SECS, ERASURE_MAX_WIDTH,
};
use tessari_s3_storage::cluster::Member;

use super::ConsoleState;
use super::error::ConsoleError;

#[derive(Serialize)]
struct MemberView {
    node: String,
    endpoint: String,
    /// Whether it answered a signed probe just now; this node answers by serving the request.
    answering: bool,
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
        Some(this) => Some(
            stream::iter(storage.cluster().members().await?)
                .map(|member| view(&state, this, member))
                .buffered(usize::from(ERASURE_MAX_WIDTH))
                .collect()
                .await,
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

/// `member` as the console shows it: this node answers by serving the request, every other member is probed.
async fn view(state: &ConsoleState, this: &str, member: Member) -> MemberView {
    let answering = member.node.as_str() == this
        || state
            .storage()
            .cluster()
            .answers(
                &member.endpoint,
                Duration::from_secs(CONSOLE_MEMBER_PROBE_SECS),
            )
            .await;
    MemberView {
        node: member.node.as_str().to_owned(),
        endpoint: member.endpoint,
        answering,
    }
}
