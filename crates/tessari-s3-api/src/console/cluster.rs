//! `GET /api/v1/cluster`: the cluster as one view — its members with whether each answers and its drive, the layout
//! objects are placed by and its erasure code, how many objects wait for healing, and the metadata nodes this node is
//! configured with. Answered only to a principal that may see the cluster; anyone else is refused, not shown less.

use axum::Json;
use axum::extract::{Extension, State};
use futures_util::stream::{self, StreamExt};
use serde::Serialize;
use tessari_s3_constants::{CONSOLE_BACKLOG_COUNT_MAX, ERASURE_MAX_WIDTH};
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::erasure::Layout;

use super::ConsoleState;
use super::access::allow;
use super::error::ConsoleError;
use super::status::{BacklogView, MemberView, view};

#[derive(Serialize)]
pub(super) struct LayoutView {
    version: u32,
    /// The erasure sets: an object goes to one, chosen by a keyed hash of its data id, one shard per node.
    sets: Vec<Vec<LayoutMemberView>>,
}

#[derive(Serialize)]
struct LayoutMemberView {
    node: String,
    /// The zone the node was in when the layout was made; absent: a domain of its own.
    zone: Option<String>,
}

impl From<&Layout> for LayoutView {
    fn from(layout: &Layout) -> Self {
        Self {
            version: layout.version(),
            sets: layout
                .sets()
                .iter()
                .map(|set| {
                    set.iter()
                        .map(|member| LayoutMemberView {
                            node: member.node().as_str().to_owned(),
                            zone: member.zone().map(|zone| zone.as_str().to_owned()),
                        })
                        .collect()
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct MetadataView {
    /// `host:port` of each metadata node, tried in turn.
    addresses: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct Cluster {
    node: Option<String>,
    erasure: Option<String>,
    /// Absent on a node on its own.
    members: Option<Vec<MemberView>>,
    /// Absent on a node on its own, and until enough members have registered.
    layout: Option<LayoutView>,
    heal_backlog: BacklogView,
    metadata: MetadataView,
}

pub(super) async fn cluster(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Cluster>, ConsoleError> {
    allow(&principal, &Action::ViewCluster)?;
    let storage = state.storage();
    let members = match state.node() {
        Some(this) => {
            let drive = Some(storage.shards().drive().await?);
            Some(
                stream::iter(storage.cluster().members().await?)
                    .map(|member| view(&state, this, member, drive))
                    .buffered(usize::from(ERASURE_MAX_WIDTH))
                    .collect()
                    .await,
            )
        }
        None => None,
    };
    let layout = match state.code() {
        Some(code) => storage
            .cluster()
            .current(code)
            .await?
            .as_ref()
            .map(LayoutView::from),
        None => None,
    };
    let backlog = storage
        .objects()
        .heal_backlog(CONSOLE_BACKLOG_COUNT_MAX)
        .await?;
    Ok(Json(Cluster {
        node: state.node().map(str::to_owned),
        erasure: state.erasure().map(str::to_owned),
        members,
        layout,
        heal_backlog: BacklogView {
            listed: backlog.as_ref().map_or(0, |found| found.listed),
            more: backlog.is_some_and(|found| found.more),
            at_risk: backlog.map_or(0, |found| found.at_risk),
        },
        metadata: MetadataView {
            addresses: state.meta_addresses().to_vec(),
        },
    }))
}
