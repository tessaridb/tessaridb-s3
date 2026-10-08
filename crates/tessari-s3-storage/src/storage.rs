//! [`Storage`]: the services the API reaches persistence through, and nothing else.

use std::path::PathBuf;

use tessari_s3_constants::DATA_BLOCK_SIZE;
use tessari_s3_infrastructure::tessaridb::MetaPool;

use crate::Result;
use crate::buckets::{BucketService, TessariBuckets};
use crate::cluster::{ClusterService, TessariCluster};
use crate::data::DataFiles;
use crate::multipart::{MultipartService, TessariMultipart};
use crate::objects::{ObjectService, TessariObjects};

/// Every storage service, built over one metadata pool.
#[derive(Clone)]
pub struct Storage {
    buckets: BucketService,
    objects: ObjectService,
    multipart: MultipartService,
    cluster: ClusterService,
    pool: MetaPool,
}

impl Storage {
    /// The services over `pool`, with data files under `data_dir` when one is configured. Nothing is dialled or
    /// created until the first call.
    #[must_use]
    pub fn new(pool: MetaPool, data_dir: Option<PathBuf>) -> Self {
        let data = data_dir.map(|root| DataFiles::new(root, DATA_BLOCK_SIZE));
        let objects = ObjectService::new(TessariObjects::new(pool.clone()), data);
        Self {
            buckets: BucketService::new(TessariBuckets::new(pool.clone())),
            multipart: MultipartService::new(TessariMultipart::new(pool.clone()), objects.clone()),
            cluster: ClusterService::new(TessariCluster::new(pool.clone())),
            objects,
            pool,
        }
    }

    /// Creates the schema when it is missing; run once at start-up.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn prepare(&self) -> Result<()> {
        crate::schema::apply(&self.pool).await
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

    /// Cluster topology: registered nodes and the layout.
    #[must_use]
    pub const fn cluster(&self) -> &ClusterService {
        &self.cluster
    }

    /// Bucket operations.
    #[must_use]
    pub const fn buckets(&self) -> &BucketService {
        &self.buckets
    }
}
