//! Topology rules: a node registers itself with its zone; the first layout is the one every node computes from the
//! same registrations — whichever proposal commits first is the layout, and a node that lost the race reads it
//! back — and `layout_current` names the version new objects are written under. Versions never change once
//! created, so each is read from the store once.

use std::sync::Arc;
use std::time::Duration;

use tessari_s3_core::erasure::{Code, Layout};
use tessari_s3_types::{NodeId, Zone};

use super::layouts::{Layouts, current_of, version_of};
use super::repository::ClusterRepository;
use super::staging::{Apply, Stage, StagedLayout, apply_of, stage_of, staged_of};
use super::tessaridb::TessariCluster;
use crate::data::DriveSpace;
use crate::peers::RemoteShards;
use crate::{Error, Result};

/// A registered node, where its internal surface answers, and its failure domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The node.
    pub node: NodeId,
    /// `host:port` of its internal surface.
    pub endpoint: String,
    /// Its zone; `None`: a domain of its own.
    pub zone: Option<Zone>,
}

/// Topology operations.
#[derive(Clone)]
pub struct ClusterService {
    repository: TessariCluster,
    layouts: Layouts,
    /// The client reaching other members, on a cluster member only.
    peers: Option<RemoteShards>,
}

impl ClusterService {
    /// The service over `repository`.
    #[must_use]
    pub fn new(repository: TessariCluster) -> Self {
        Self {
            repository,
            layouts: Arc::default(),
            peers: None,
        }
    }

    /// The service of a cluster member, reaching the other members through `peers`.
    #[must_use]
    pub(crate) fn with_peers(self, peers: RemoteShards) -> Self {
        Self {
            peers: Some(peers),
            ..self
        }
    }

    /// The data drive space of the member at `endpoint`, when it answers within `budget` and stores data; `None`
    /// otherwise, including off a cluster.
    pub async fn drive(&self, endpoint: &str, budget: Duration) -> Option<DriveSpace> {
        let peers = self.peers.as_ref()?;
        match peers.drive(endpoint, budget).await {
            Ok(drive) => drive,
            Err(error) => {
                tracing::debug!(endpoint, error = %error, "a member's drive was not read");
                None
            }
        }
    }

    /// Whether the member at `endpoint` answers a signed probe within `budget`. A node outside a cluster reaches
    /// nobody, so it is always `false` there.
    pub async fn answers(&self, endpoint: &str, budget: Duration) -> bool {
        match &self.peers {
            Some(peers) => peers.probe(endpoint, budget).await.is_ok(),
            None => false,
        }
    }

    /// Registers `node` at `endpoint` in `zone`, or refreshes its registration.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn register(&self, node: &NodeId, endpoint: &str, zone: Option<&Zone>) -> Result<()> {
        self.repository
            .register(node.as_str(), endpoint, zone.map(Zone::as_str))
            .await
    }

    /// Registered nodes in id order.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn members(&self) -> Result<Vec<Member>> {
        members(&self.repository).await
    }

    /// The layout new objects are written under: the one the current version names, or the first one once enough
    /// nodes in enough zones have registered (`None` until then).
    ///
    /// # Errors
    /// [`crate::Error::LayoutMismatch`] when a stored layout uses another code; the store's refusal or outage.
    pub async fn current(&self, code: Code) -> Result<Option<Layout>> {
        current_of(&self.repository, &self.layouts, code).await
    }

    /// Layout `version`, which objects written under it name.
    ///
    /// # Errors
    /// [`crate::Error::Malformed`] when no such layout exists; [`crate::Error::LayoutMismatch`]; the store's refusal
    /// or outage.
    pub async fn version(&self, code: Code, version: u32) -> Result<Layout> {
        version_of(&self.repository, &self.layouts, code, version).await
    }

    /// The layout an operator staged, waiting to be applied.
    ///
    /// # Errors
    /// [`crate::Error::Malformed`] for a staged record this node cannot read; the store's refusal or outage.
    pub async fn staged(&self, code: Code) -> Result<Option<StagedLayout>> {
        staged_of(&self.repository, code).await
    }

    /// Stages `sets` of registered nodes as the next layout, replacing anything staged before.
    ///
    /// # Errors
    /// [`crate::Error::LayoutMismatch`] when the current layout uses another code; the store's refusal or outage.
    pub async fn stage(&self, code: Code, sets: Vec<Vec<NodeId>>) -> Result<Stage> {
        stage_of(&self.repository, &self.layouts, code, sets).await
    }

    /// Discards the staged layout, if any.
    ///
    /// # Errors
    /// The store's refusal or outage.
    pub async fn unstage(&self) -> Result<()> {
        self.repository.unstage().await
    }

    /// Applies the staged layout named `id` as the next version.
    ///
    /// # Errors
    /// [`crate::Error::Malformed`] for a staged record this node cannot read; the store's refusal or outage.
    pub async fn apply(&self, code: Code, id: &str) -> Result<Apply> {
        apply_of(&self.repository, &self.layouts, code, id).await
    }
}

/// Registered nodes in id order.
pub(super) async fn members(repository: &impl ClusterRepository) -> Result<Vec<Member>> {
    let malformed = |reason| Error::Malformed {
        record: "node",
        reason,
    };
    repository
        .nodes()
        .await?
        .into_iter()
        .map(|entity| {
            Ok(Member {
                node: NodeId::new(&entity.node).map_err(|_| malformed("node id"))?,
                endpoint: entity.endpoint,
                zone: entity
                    .zone
                    .map(|zone| Zone::new(&zone).map_err(|_| malformed("zone")))
                    .transpose()?,
            })
        })
        .collect()
}
