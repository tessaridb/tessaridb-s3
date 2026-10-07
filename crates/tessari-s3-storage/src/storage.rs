//! [`Storage`]: the services the API reaches persistence through, and nothing else.

use std::path::PathBuf;

use tessari_s3_constants::DATA_BLOCK_SIZE;
use tessari_s3_infrastructure::tessaridb::MetaPool;

use crate::Result;
use crate::buckets::{BucketService, TessariBuckets};
use crate::data::DataFiles;
use crate::objects::{ObjectService, TessariObjects};

/// Every storage service, built over one metadata pool.
#[derive(Clone)]
pub struct Storage {
    buckets: BucketService,
    objects: ObjectService,
    pool: MetaPool,
}

impl Storage {
    /// The services over `pool`, with data files under `data_dir` when one is configured. Nothing is dialled or
    /// created until the first call.
    #[must_use]
    pub fn new(pool: MetaPool, data_dir: Option<PathBuf>) -> Self {
        let data = data_dir.map(|root| DataFiles::new(root, DATA_BLOCK_SIZE));
        Self {
            buckets: BucketService::new(TessariBuckets::new(pool.clone())),
            objects: ObjectService::new(TessariObjects::new(pool.clone()), data),
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

    /// Bucket operations.
    #[must_use]
    pub const fn buckets(&self) -> &BucketService {
        &self.buckets
    }
}
