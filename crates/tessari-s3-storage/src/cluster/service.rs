//! Topology rules: a node registers itself, and the first layout is the one every node computes from the same
//! registrations — whichever proposal commits first is the layout, and a node that lost the race reads it back.

use tessari_s3_core::erasure::{Code, Layout};
use tessari_s3_types::NodeId;

use super::entity::LayoutEntity;
use super::repository::{ClusterRepository, Proposed};
use super::tessaridb::TessariCluster;
use crate::{Error, Result};

/// A registered node and where its internal surface answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The node.
    pub node: NodeId,
    /// `host:port` of its internal surface.
    pub endpoint: String,
}

/// Topology operations.
#[derive(Clone)]
pub struct ClusterService {
    repository: TessariCluster,
}

impl ClusterService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariCluster) -> Self {
        Self { repository }
    }

    /// Registers `node` at `endpoint`, or refreshes its registration.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn register(&self, node: &NodeId, endpoint: &str) -> Result<()> {
        self.repository.register(node.as_str(), endpoint).await
    }

    /// Registered nodes in id order.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn members(&self) -> Result<Vec<Member>> {
        members(&self.repository).await
    }

    /// The cluster's layout under `code`: the stored one, or the first one once enough nodes have registered
    /// (`None` until then).
    ///
    /// # Errors
    /// [`crate::Error::LayoutMismatch`] when the stored layout uses another code; the store's refusal or outage.
    pub async fn layout(&self, code: Code) -> Result<Option<Layout>> {
        layout_of(&self.repository, code).await
    }
}

/// Registered nodes in id order.
async fn members(repository: &impl ClusterRepository) -> Result<Vec<Member>> {
    repository
        .nodes()
        .await?
        .into_iter()
        .map(|entity| {
            let node = NodeId::new(&entity.node).map_err(|_| Error::Malformed {
                record: "node",
                reason: "node id",
            })?;
            Ok(Member {
                node,
                endpoint: entity.endpoint,
            })
        })
        .collect()
}

/// The stored first layout, or this node's proposal for it; a proposal that lost the race reads back the winner.
async fn layout_of(repository: &impl ClusterRepository, code: Code) -> Result<Option<Layout>> {
    if let Some(stored) = repository.layout(FIRST_VERSION).await? {
        return held(&stored, code).map(Some);
    }
    let members = members(repository).await?;
    let Some(proposal) = Layout::first(code, members.into_iter().map(|member| member.node)) else {
        return Ok(None);
    };
    let entity = LayoutEntity {
        version: FIRST_VERSION,
        data: i64::from(code.data()),
        parity: i64::from(code.parity()),
        nodes: proposal
            .nodes()
            .iter()
            .map(|node| node.as_str().to_owned())
            .collect(),
    };
    match repository.propose(&entity).await? {
        Proposed::Created => Ok(Some(proposal)),
        // Another node's proposal committed first; it is the layout, whatever this node computed.
        Proposed::Exists => match repository.layout(FIRST_VERSION).await? {
            Some(stored) => held(&stored, code).map(Some),
            None => Err(Error::Malformed {
                record: "layout",
                reason: "refused as existing, then not found",
            }),
        },
    }
}

/// The only version SG5 creates; later versions are an operator's act.
const FIRST_VERSION: i64 = 1;

/// A stored layout as this node may use it: under the code it is configured with, and well formed.
fn held(stored: &LayoutEntity, code: Code) -> Result<Layout> {
    if stored.data != i64::from(code.data()) || stored.parity != i64::from(code.parity()) {
        return Err(Error::LayoutMismatch {
            stored_data: stored.data,
            stored_parity: stored.parity,
            data: code.data(),
            parity: code.parity(),
        });
    }
    let malformed = |reason| Error::Malformed {
        record: "layout",
        reason,
    };
    let version = u32::try_from(stored.version).map_err(|_| malformed("version"))?;
    let nodes = stored
        .nodes
        .iter()
        .map(|id| NodeId::new(id).map_err(|_| malformed("node id")))
        .collect::<Result<Vec<_>>>()?;
    Layout::new(version, code, nodes).map_err(|_| malformed("shape"))
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
