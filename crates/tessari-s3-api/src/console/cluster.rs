//! `GET /api/v1/cluster`: the cluster as one view — its members with whether each answers and its drive, the layout
//! objects are placed by and its erasure code, how many objects wait for healing, and the metadata nodes this node is
//! configured with. Answered only to a principal that may see the cluster; anyone else is refused, not shown less.

use axum::Json;
use axum::extract::{Extension, State};
use futures_util::stream::{self, StreamExt};
use serde::Serialize;
use tessari_s3_constants::{CONSOLE_BACKLOG_COUNT_MAX, ERASURE_MAX_WIDTH};
use tessari_s3_core::authz::{Action, Principal};

use super::ConsoleState;
use super::access::allow;
use super::error::ConsoleError;
use super::status::{BacklogView, MemberView, view};

#[derive(Serialize)]
struct LayoutView {
    version: u32,
    /// The nodes in placement order: shard `i` of every object goes to the `i`-th.
    nodes: Vec<String>,
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
            let drive = storage.shards().drive().await?;
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
            .layout(code)
            .await?
            .map(|layout| LayoutView {
                version: layout.version(),
                nodes: layout
                    .nodes()
                    .iter()
                    .map(|node| node.as_str().to_owned())
                    .collect(),
            }),
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
        },
        metadata: MetadataView {
            addresses: state.meta_addresses().to_vec(),
        },
    }))
}
