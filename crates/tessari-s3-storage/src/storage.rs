//! [`Storage`]: the services the API reaches persistence through, and nothing else.

use std::path::PathBuf;
use std::sync::Arc;

use tessari_s3_constants::DATA_BLOCK_SIZE;
use tessari_s3_infrastructure::ClusterSettings;
use tessari_s3_infrastructure::tessaridb::MetaPool;

use crate::Result;
use crate::actions::{ActionService, TessariActions};
use crate::buckets::{BucketService, TessariBuckets};
use crate::cluster::{ClusterService, TessariCluster};
use crate::data::DataFiles;
use crate::erasure::ErasureWrites;
use crate::multipart::{MultipartService, TessariMultipart};
use crate::objects::{ObjectService, TessariObjects};
use crate::peers::RemoteShards;
use crate::shards::ShardService;
use crate::usage::{TessariUsage, UsageService};

/// Every storage service, built over one metadata pool.
#[derive(Clone)]
pub struct Storage {
    buckets: BucketService,
    objects: ObjectService,
    multipart: MultipartService,
    cluster: ClusterService,
    shards: ShardService,
    actions: ActionService,
    usage: UsageService,
    pool: MetaPool,
}

impl Storage {
    /// The services over `pool`, with data files under `data_dir` when one is configured. Nothing is dialled or
    /// created until the first call.
    #[must_use]
    pub fn new(pool: MetaPool, data_dir: Option<PathBuf>) -> Self {
        let data = data_dir.map(|root| DataFiles::new(root, DATA_BLOCK_SIZE));
        let shards = ShardService::new(data.clone());
        let objects = ObjectService::new(TessariObjects::new(pool.clone()), data);
        Self {
            buckets: BucketService::new(TessariBuckets::new(pool.clone())),
            multipart: MultipartService::new(TessariMultipart::new(pool.clone()), objects.clone()),
            cluster: ClusterService::new(TessariCluster::new(pool.clone())),
            actions: ActionService::new(TessariActions::new(pool.clone())),
            usage: UsageService::new(TessariUsage::new(pool.clone())),
            objects,
            shards,
            pool,
        }
    }

    /// The services of a cluster member: as [`Storage::new`], with every data id written as erasure-coded shards
    /// over the cluster described by `cluster`.
    ///
    /// # Errors
    /// [`crate::Error::Peer`] when the peer client cannot be built.
    pub fn clustered(
        pool: MetaPool,
        data_dir: Option<PathBuf>,
        cluster: &ClusterSettings,
    ) -> Result<Self> {
        let mut storage = Self::new(pool.clone(), data_dir);
        let peers = RemoteShards::new(
            cluster.node.clone(),
            cluster.secret.clone(),
            cluster.tls.as_ref(),
        )?;
        storage.cluster = storage.cluster.clone().with_peers(peers.clone());
        let writes = ErasureWrites::new(
            cluster,
            pool.clone(),
            storage.cluster.clone(),
            storage.shards.clone(),
            peers,
        );
        storage.objects = storage.objects.clone().clustered(Arc::new(writes));
        storage.multipart =
            MultipartService::new(TessariMultipart::new(pool), storage.objects.clone());
        Ok(storage)
    }

    /// Creates the schema when it is missing and clears what writes cut short by a crash left on the drive; run once
    /// at start-up, before the node serves.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or the drive's.
    pub async fn prepare(&self) -> Result<()> {
        crate::schema::apply(&self.pool).await?;
        let cleared = self.objects.recover().await?;
        if cleared > 0 {
            tracing::info!(cleared, "temporary files of interrupted writes removed");
        }
        Ok(())
    }

    /// Object operations.
    #[must_use]
    pub const fn objects(&self) -> &ObjectService {
        &self.objects
    }

    /// Multipart upload operations.
    #[must_use]
    pub const fn multipart(&self) -> &MultipartService {
        &self.multipart
    }

    /// Cluster topology: registered nodes, whether they answer, and the layout.
    #[must_use]
    pub const fn cluster(&self) -> &ClusterService {
        &self.cluster
    }

    /// This node's shards of erasure-coded objects.
    #[must_use]
    pub const fn shards(&self) -> &ShardService {
        &self.shards
    }

    /// Bucket operations.
    #[must_use]
    pub const fn buckets(&self) -> &BucketService {
        &self.buckets
    }

    /// The record of what operators did through the console.
    #[must_use]
    pub const fn actions(&self) -> &ActionService {
        &self.actions
    }

    /// How many objects and bytes each bucket holds, as last measured.
    #[must_use]
    pub const fn usage(&self) -> &UsageService {
        &self.usage
    }
}
