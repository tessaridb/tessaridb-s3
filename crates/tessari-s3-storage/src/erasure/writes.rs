//! [`ErasureWrites`]: a cluster member's data plane — where each shard of a data id goes, starting an erasure
//! upload after its id is queued with its layout, and removing every shard of an id from every node.

use std::sync::Arc;
use std::time::Duration;

use tessari_s3_constants::{DATA_BLOCK_SIZE, SHARD_STALL_SECS};
use tessari_s3_core::erasure::{Code, Layout, PlacementKey, placement};
use tessari_s3_infrastructure::ClusterSettings;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_types::NodeId;

use super::read::ErasureReader;
use super::repository::{ShardSet, ShardSetRepository};
use super::tessaridb::TessariShardSets;
use super::upload::{Destination, ErasurePlan, ErasureUpload};
use crate::cluster::ClusterService;
use crate::peers::{PeerAt, RemoteShards, ShardRef};
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

    /// The layout new objects are written under, read from the store for every write so a change applied on any
    /// node is followed by the next write here.
    pub(super) async fn current(&self) -> Result<Layout> {
        self.cluster
            .current(self.code)
            .await?
            .ok_or(Error::NoLayout)
    }

    /// Layout `version`, which data written under it names.
    pub(super) async fn version(&self, version: u32) -> Result<Layout> {
        self.cluster.version(self.code, version).await
    }

    /// The node each shard of data `id` goes to under `layout`, by shard index.
    pub(super) fn placed(&self, id: [u8; 16], layout: &Layout) -> Vec<NodeId> {
        placement(&self.key, id, layout)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Where each shard of data `id` goes under `layout`: this node's drive, or the registered endpoint of a peer.
    pub(super) async fn destinations(
        &self,
        id: [u8; 16],
        layout: &Layout,
    ) -> Result<Vec<Destination>> {
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
                    .map(|member| {
                        Destination::Remote(PeerAt {
                            node: member.node.clone(),
                            endpoint: member.endpoint.clone(),
                        })
                    })
                    .ok_or(Error::Malformed {
                        record: "s3_nodes",
                        reason: "a layout node is not registered",
                    })
            })
            .collect()
    }

    /// Queues data `id` with its layout, then starts writing its `size` bytes as shards, sealed shards when `sealed`.
    ///
    /// # Errors
    /// [`Error::NoLayout`], the metadata store's refusal or outage, or a plan that does not fit the code.
    pub(crate) async fn start(
        &self,
        id: [u8; 16],
        size: u64,
        sealed: bool,
    ) -> Result<ErasureUpload> {
        let layout = self.current().await?;
        let set = ShardSet {
            layout: layout.version(),
            size,
            sealed,
        };
        self.sets.queue(id, set).await?;
        let destinations = self.destinations(id, &layout).await?;
        let plan = ErasurePlan {
            id,
            code: self.code,
            stripe_size: DATA_BLOCK_SIZE,
            size,
            destinations,
            stall: self.stall,
            sealed,
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

    /// A reader of data `id` holding `size` stored bytes, sealed ones when `sealed`, when it is erasure-coded; `None`
    /// when it is a whole file. The shard set must say the same: plain shards are never read as sealed bytes, nor
    /// sealed shards as plain ones.
    ///
    /// # Errors
    /// [`Error::Corrupt`] when the shard set disagrees on the size or on sealing, [`Error::Malformed`] for a layout
    /// this node does not hold, [`Error::NoLayout`], or the store's refusal.
    pub(crate) async fn open(
        self: &Arc<Self>,
        id: [u8; 16],
        size: u64,
        sealed: bool,
    ) -> Result<Option<ErasureReader>> {
        let Some((set, sources)) = self.sources_of(id).await? else {
            return Ok(None);
        };
        if set.size != size || set.sealed != sealed {
            return Err(Error::Corrupt {
                id: crate::data::hex(id),
                reason: "the shard set does not hold what the object names",
            });
        }
        ErasureReader::new(Arc::clone(self), id, set, sources).map(Some)
    }

    /// Data `id`'s shard set and where each of its shards is under the layout it names, when it is erasure-coded.
    ///
    /// # Errors
    /// [`Error::Malformed`] for a layout version nothing created, or the store's refusal.
    pub(super) async fn sources_of(
        &self,
        id: [u8; 16],
    ) -> Result<Option<(ShardSet, Vec<Destination>)>> {
        let Some(set) = self.sets.shard_set(id).await? else {
            return Ok(None);
        };
        let layout = self.version(set.layout).await?;
        Ok(Some((set, self.destinations(id, &layout).await?)))
    }

    /// Lists data `id` for healing: it was acknowledged with fewer than every shard durable.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub(crate) async fn heal(&self, id: [u8; 16]) -> Result<()> {
        self.sets.heal(id).await
    }

    /// Removes every shard of data `id` from every node, then its shard-set record, under this node's claim on the
    /// id so no healer or mover writes a shard behind it. The layout is read under the claim: one read before it may
    /// already have been moved on. A node that cannot be reached leaves the record, so a later pass tries again.
    ///
    /// # Errors
    /// [`Error::Held`] while another node works on the id, [`Error::Malformed`] for a layout version nothing
    /// created, a peer's refusal or absence, or the store's.
    pub(crate) async fn remove(&self, id: [u8; 16]) -> Result<()> {
        if !self.sets.claim(id, self.holder()).await? {
            return Err(Error::Held);
        }
        let removed = self.remove_claimed(id).await;
        self.unclaim_logged(id).await;
        removed
    }

    async fn remove_claimed(&self, id: [u8; 16]) -> Result<()> {
        let Some(set) = self.sets.shard_set(id).await? else {
            return Ok(());
        };
        let layout = self.version(set.layout).await?;
        for (index, destination) in self
            .destinations(id, &layout)
            .await?
            .into_iter()
            .enumerate()
        {
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
