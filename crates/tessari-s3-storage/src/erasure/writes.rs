//! [`ErasureWrites`]: a cluster member's data plane — where each shard of a data id goes, starting an erasure
//! upload after its id is queued with its layout, and removing every shard of an id from every node.

use std::sync::Arc;
use std::time::Duration;

use tessari_s3_constants::{DATA_BLOCK_SIZE, SHARD_STALL_SECS};
use tessari_s3_core::erasure::{Code, Layout, PlacementKey, placement};
use tessari_s3_infrastructure::ClusterSettings;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_types::NodeId;
use tokio::sync::OnceCell;

use super::read::ErasureReader;
use super::repository::{ShardSet, ShardSetRepository};
use super::tessaridb::TessariShardSets;
use super::upload::{Destination, ErasurePlan, ErasureUpload};
use crate::cluster::ClusterService;
use crate::peers::{RemoteShards, ShardRef};
use crate::shards::ShardService;
use crate::{Error, Result};

/// This node's view of the cluster's data plane.
pub(crate) struct ErasureWrites {
    node: NodeId,
    code: Code,
    key: PlacementKey,
    cluster: ClusterService,
    shards: ShardService,
    peers: RemoteShards,
    pub(super) sets: TessariShardSets,
    stall: Duration,
    /// Layouts never change once created, so the first one read is kept.
    layout: OnceCell<Layout>,
}

impl ErasureWrites {
    /// The data plane of the node `settings` describe, reaching the other nodes through `peers`.
    pub(crate) fn new(
        settings: &ClusterSettings,
        pool: MetaPool,
        cluster: ClusterService,
        shards: ShardService,
        peers: RemoteShards,
    ) -> Self {
        Self {
            node: settings.node.clone(),
            code: settings.code,
            key: PlacementKey::derive(settings.secret.expose().as_bytes()),
            cluster,
            shards,
            peers,
            sets: TessariShardSets::new(pool),
            stall: Duration::from_secs(SHARD_STALL_SECS),
            layout: OnceCell::new(),
        }
    }

    pub(super) const fn code(&self) -> Code {
        self.code
    }

    pub(super) const fn shards(&self) -> &ShardService {
        &self.shards
    }

    pub(super) const fn peers(&self) -> &RemoteShards {
        &self.peers
    }

    pub(super) const fn stall(&self) -> Duration {
        self.stall
    }

    /// The name this node's claims are held under.
    pub(super) fn holder(&self) -> &str {
        self.node.as_str()
    }

    /// Hands back this node's claim on data `id`; a failure only costs the claim's remaining lease.
    pub(super) async fn unclaim_logged(&self, id: [u8; 16]) {
        if let Err(error) = self.sets.unclaim(id, self.holder()).await {
            tracing::warn!(data = %crate::data::hex(id), error = %error, "a claim is left to lapse");
        }
    }

    async fn layout(&self) -> Result<&Layout> {
        self.layout
            .get_or_try_init(|| async {
                self.cluster.layout(self.code).await?.ok_or(Error::NoLayout)
            })
            .await
    }

    /// Where each shard of data `id` goes under `layout`: this node's drive, or the registered endpoint of a peer.
    async fn destinations(&self, id: [u8; 16], layout: &Layout) -> Result<Vec<Destination>> {
        let members = self.cluster.members().await?;
        placement(&self.key, id, layout)
            .into_iter()
            .map(|node| {
                if *node == self.node {
                    return Ok(Destination::Local);
                }
                members
                    .iter()
                    .find(|member| member.node == *node)
                    .map(|member| Destination::Remote(member.endpoint.clone()))
                    .ok_or(Error::Malformed {
                        record: "s3_nodes",
                        reason: "a layout node is not registered",
                    })
            })
            .collect()
    }

    /// Queues data `id` with its layout, then starts writing its `size` bytes as shards.
    ///
    /// # Errors
    /// [`Error::NoLayout`], the metadata store's refusal or outage, or a plan that does not fit the code.
    pub(crate) async fn start(&self, id: [u8; 16], size: u64) -> Result<ErasureUpload> {
        let layout = self.layout().await?;
        let set = ShardSet {
            layout: layout.version(),
            size,
        };
        self.sets.queue(id, set).await?;
        let destinations = self.destinations(id, layout).await?;
        let plan = ErasurePlan {
            id,
            code: self.code,
            stripe_size: DATA_BLOCK_SIZE,
            size,
            destinations,
            stall: self.stall,
        };
        ErasureUpload::start(plan, &self.shards, &self.peers)
    }

    /// The layout data `id` was written under, when it is erasure-coded.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub(crate) async fn sharded(&self, id: [u8; 16]) -> Result<Option<u32>> {
        Ok(self.sets.shard_set(id).await?.map(|set| set.layout))
    }

    /// A reader of data `id` holding `size` bytes when it is erasure-coded, `None` when it is a whole file.
    ///
    /// # Errors
    /// [`Error::Malformed`] for a layout this node does not hold, [`Error::NoLayout`], or the store's refusal.
    pub(crate) async fn open(
        self: &Arc<Self>,
        id: [u8; 16],
        size: u64,
    ) -> Result<Option<ErasureReader>> {
        let Some((_, sources)) = self.sources_of(id).await? else {
            return Ok(None);
        };
        ErasureReader::new(Arc::clone(self), id, size, sources).map(Some)
    }

    /// Data `id`'s shard set and where each of its shards is, when it is erasure-coded.
    ///
    /// # Errors
    /// [`Error::Malformed`] for a layout this node does not hold, [`Error::NoLayout`], or the store's refusal.
    pub(super) async fn sources_of(
        &self,
        id: [u8; 16],
    ) -> Result<Option<(ShardSet, Vec<Destination>)>> {
        let Some(set) = self.sets.shard_set(id).await? else {
            return Ok(None);
        };
        let layout = self.layout().await?;
        if layout.version() != set.layout {
            return Err(Error::Malformed {
                record: "shard_sets",
                reason: "a layout this node does not hold",
            });
        }
        Ok(Some((set, self.destinations(id, layout).await?)))
    }

    /// Lists data `id` for healing: it was acknowledged with fewer than every shard durable.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub(crate) async fn heal(&self, id: [u8; 16]) -> Result<()> {
        self.sets.heal(id).await
    }

    /// Removes every shard of data `id` — written under layout `version` — from every node, then its shard-set
    /// record, under this node's claim on the id so no healer writes a shard back behind it. A node that cannot be
    /// reached leaves the record, so a later pass tries again.
    ///
    /// # Errors
    /// [`Error::Held`] while another node works on the id, [`Error::Malformed`] for a layout this node does not
    /// hold, a peer's refusal or absence, or the store's.
    pub(crate) async fn remove(&self, id: [u8; 16], version: u32) -> Result<()> {
        if !self.sets.claim(id, self.holder()).await? {
            return Err(Error::Held);
        }
        let removed = self.remove_claimed(id, version).await;
        self.unclaim_logged(id).await;
        removed
    }

    async fn remove_claimed(&self, id: [u8; 16], version: u32) -> Result<()> {
        let layout = self.layout().await?;
        if layout.version() != version {
            return Err(Error::Malformed {
                record: "shard_sets",
                reason: "a layout this node does not hold",
            });
        }
        for (index, destination) in self.destinations(id, layout).await?.into_iter().enumerate() {
            let shard = ShardRef {
                id,
                index: u16::try_from(index).map_err(|_| Error::InvalidShard {
                    reason: "too many shards",
                })?,
            };
            match destination {
                Destination::Local => self.shards.remove(id, shard.index).await?,
                Destination::Remote(endpoint) => {
                    self.peers.delete(&endpoint, shard, self.stall).await?;
                }
            }
        }
        self.sets.forget(id).await
    }
}
