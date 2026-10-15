//! `GET /api/v1/status`: what this node is — its name in the cluster, the build, the region it signs for, the space of
//! its data drive — and, on a cluster member, the registered members with whether each answers now and its drive's
//! space, how many objects wait for healing, and how this region's replication stands. The drive, the members and the backlog are the cluster view, answered
//! only to a principal that may see the cluster.

use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, State};
use futures_util::stream::{self, StreamExt};
use serde::Serialize;
use tessari_s3_constants::{
    CONSOLE_BACKLOG_COUNT_MAX, CONSOLE_MEMBER_PROBE_SECS, ERASURE_MAX_WIDTH,
};
use tessari_s3_core::authz::{Action, Decision, Principal, authorize};
use tessari_s3_storage::cluster::Member;
use tessari_s3_storage::data::DriveSpace;

use super::ConsoleState;
use super::error::ConsoleError;

/// A data drive's space in bytes.
#[derive(Serialize)]
pub(super) struct DriveView {
    capacity: u64,
    free: u64,
    available: u64,
}

impl From<DriveSpace> for DriveView {
    fn from(space: DriveSpace) -> Self {
        Self {
            capacity: space.capacity,
            free: space.free,
            available: space.available,
        }
    }
}

#[derive(Serialize)]
pub(super) struct MemberView {
    node: String,
    endpoint: String,
    /// Its failure domain; absent when it names none and is a domain of its own.
    zone: Option<String>,
    /// Whether it answered a signed probe just now; this node answers by serving the request.
    answering: bool,
    /// Its data drive's space; absent when it did not answer or stores no data.
    drive: Option<DriveView>,
}

#[derive(Serialize)]
pub(super) struct BacklogView {
    /// Objects listed for healing, counted up to the bound.
    pub(super) listed: usize,
    /// Whether more are listed than were counted.
    pub(super) more: bool,
    /// Of those counted, the objects whose healing has failed often enough that it will not succeed by itself.
    pub(super) at_risk: usize,
}

/// Where this region's replication stands.
#[derive(Serialize)]
pub(super) struct ReplicationView {
    /// Versions not yet sent.
    backlog: u64,
    /// Versions being sent now.
    in_flight: u64,
    /// Versions given up (`FAILED`).
    failed: u64,
}

#[derive(Serialize)]
pub(super) struct Status {
    version: &'static str,
    node: Option<String>,
    region: String,
    erasure: Option<String>,
    members: Option<Vec<MemberView>>,
    heal_backlog: Option<BacklogView>,
    replication: Option<ReplicationView>,
    /// This node's data drive; absent on a node without a data directory.
    drive: Option<DriveView>,
}

pub(super) async fn status(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Status>, ConsoleError> {
    // The members, their drives and the heal backlog are the cluster view: shown only to who may see the cluster.
    if authorize(&principal, &Action::ViewCluster) != Decision::Allow {
        return Ok(Json(Status {
            version: env!("CARGO_PKG_VERSION"),
            node: state.node().map(str::to_owned),
            region: state.region().to_owned(),
            erasure: state.erasure().map(str::to_owned),
            members: None,
            heal_backlog: None,
            replication: None,
            drive: None,
        }));
    }
    let storage = state.storage();
    let drive = Some(storage.shards().drive().await?);
    let members = match state.node() {
        Some(this) => Some(
            stream::iter(storage.cluster().members().await?)
                .map(|member| view(&state, this, member, drive))
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
            at_risk: backlog.at_risk,
        });
    // The node's own status answers while the metadata store does not; replication is then left out, and said so.
    let replication = match storage.replication().progress().await {
        Ok(progress) => Some(ReplicationView {
            backlog: progress.backlog,
            in_flight: progress.in_flight,
            failed: progress.failed,
        }),
        Err(error) => {
            tracing::warn!(%error, "replication progress could not be read");
            None
        }
    };
    Ok(Json(Status {
        version: env!("CARGO_PKG_VERSION"),
        node: state.node().map(str::to_owned),
        region: state.region().to_owned(),
        erasure: state.erasure().map(str::to_owned),
        members,
        heal_backlog,
        replication,
        drive: drive.map(DriveView::from),
    }))
}

/// `member` as the console shows it: this node answers by serving the request and reports `local` as its drive;
/// every other member is probed and asked for its drive.
pub(super) async fn view(
    state: &ConsoleState,
    this: &str,
    member: Member,
    local: Option<DriveSpace>,
) -> MemberView {
    let budget = Duration::from_secs(CONSOLE_MEMBER_PROBE_SECS);
    let cluster = state.storage().cluster();
    let (answering, drive) = if member.node.as_str() == this {
        (true, local)
    } else {
        let drive = cluster.drive(&member, budget).await;
        // A member that reported its drive answered; one with no data directory still answers its probe.
        let answering = drive.is_some() || cluster.answers(&member, budget).await;
        (answering, drive)
    };
    MemberView {
        node: member.node.as_str().to_owned(),
        endpoint: member.endpoint,
        zone: member.zone.map(|zone| zone.as_str().to_owned()),
        answering,
        drive: drive.map(DriveView::from),
    }
}
