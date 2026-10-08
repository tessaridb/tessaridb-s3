//! [`ErasureWrites`]: a cluster member's data plane — where each shard of a data id goes, starting an erasure
//! upload after its id is queued with its layout, and removing every shard of an id from every node.

use std::time::Duration;

use tessari_s3_constants::{DATA_BLOCK_SIZE, SHARD_STALL_SECS};
use tessari_s3_core::erasure::{Code, Layout, PlacementKey, placement};
use tessari_s3_infrastructure::ClusterSettings;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_types::NodeId;
use tokio::sync::OnceCell;

use super::repository::ShardSetRepository;
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
    sets: TessariShardSets,
    stall: Duration,
    /// Layouts never change once created, so the first one read is kept.
    layout: OnceCell<Layout>,
}

impl ErasureWrites {
    /// The data plane of the node `settings` describe.
    ///
    /// # Errors
    /// [`Error::Peer`] when the peer client cannot be built.
    pub(crate) fn new(
        settings: &ClusterSettings,
        pool: MetaPool,
        cluster: ClusterService,
        shards: ShardService,
    ) -> Result<Self> {
        Ok(Self {
            node: settings.node.clone(),
            code: settings.code,
            key: PlacementKey::derive(settings.secret.expose().as_bytes()),
            cluster,
            shards,
            peers: RemoteShards::new(settings.node.clone(), settings.secret.clone())?,
            sets: TessariShardSets::new(pool),
            stall: Duration::from_secs(SHARD_STALL_SECS),
            layout: OnceCell::new(),
        })
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
        self.sets.queue(id, layout.version()).await?;
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
        self.sets.layout_of(id).await
    }

    /// Lists data `id` for healing: it was acknowledged with fewer than every shard durable.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub(crate) async fn heal(&self, id: [u8; 16]) -> Result<()> {
        self.sets.heal(id).await
    }

    /// Removes every shard of data `id` — written under layout `version` — from every node, then its shard-set
    /// record. A node that cannot be reached leaves the record, so a later pass tries again.
    ///
    /// # Errors
    /// [`Error::Malformed`] for a layout this node does not hold, a peer's refusal or absence, or the store's.
    pub(crate) async fn remove(&self, id: [u8; 16], version: u32) -> Result<()> {
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
