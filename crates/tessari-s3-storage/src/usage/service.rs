//! Usage rules: one member measures at a time, by a claim the store expires, and the console reads what was stored.

use tessari_s3_constants::DATA_BLOCK_SIZE;
use tessari_s3_core::erasure::Stripes;
use tessari_s3_types::Code;

use super::entity::BucketUsageEntity;
use super::model::Usage;
use super::occupancy::{Geometry, Occupancy, occupancy};
use super::repository::UsageRepository;
use super::tessaridb::TessariUsage;
use crate::{Error, Result};

/// Usage operations.
#[derive(Clone)]
pub struct UsageService {
    repository: TessariUsage,
    geometry: Option<Geometry>,
}

impl UsageService {
    /// The service over `repository`, for a node storing data files as they are.
    #[must_use]
    pub const fn new(repository: TessariUsage) -> Self {
        Self {
            repository,
            geometry: None,
        }
    }

    /// The same service on a cluster writing erasure `code`: bytes on the drives count every stripe at its full
    /// width.
    ///
    /// # Errors
    /// [`crate::Error::Malformed`] when the code and the stripe size give no block length.
    pub fn clustered(self, code: Code) -> Result<Self> {
        let stripes = Stripes::new(code, DATA_BLOCK_SIZE).map_err(|_| Error::Malformed {
            record: "layouts",
            reason: "an erasure code with no block length",
        })?;
        let block_len = u64::try_from(stripes.block_len()).map_err(|_| Error::Malformed {
            record: "layouts",
            reason: "a block length past 64 bits",
        })?;
        Ok(Self {
            geometry: Some(Geometry {
                block_len,
                width: u64::from(code.width()),
            }),
            ..self
        })
    }

    /// Measures every bucket and stores the result, unless another member holds the claim. Answers how many buckets
    /// were measured, or `None` when this pass was another member's.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed answer.
    pub async fn refresh(&self, holder: &str) -> Result<Option<usize>> {
        refresh(&self.repository, holder, self.geometry).await
    }

    /// The latest measurement, if any was taken.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn latest(&self) -> Result<Option<Usage>> {
        self.repository
            .latest()
            .await?
            .map(Usage::try_from)
            .transpose()
    }
}

pub(super) async fn refresh(
    repository: &impl UsageRepository,
    holder: &str,
    geometry: Option<Geometry>,
) -> Result<Option<usize>> {
    if !repository.claim(holder).await? {
        return Ok(None);
    }
    let measured = repository.measure().await?;
    let buckets = occupancy(
        &measured.objects,
        &measured.parts,
        &measured.owners,
        geometry,
    )
    .into_iter()
    .map(entity)
    .collect::<Result<Vec<_>>>()?;
    repository.store(&buckets).await?;
    Ok(Some(buckets.len()))
}

/// The stored form of one bucket's figures; the store holds signed 64-bit integers.
fn entity(figures: Occupancy) -> Result<BucketUsageEntity> {
    let stored = |value: u64, reason| {
        i64::try_from(value).map_err(|_| Error::Malformed {
            record: "bucket_usage",
            reason,
        })
    };
    Ok(BucketUsageEntity {
        objects: stored(figures.objects, "objects")?,
        bytes: stored(figures.bytes, "bytes")?,
        inline_bytes: stored(figures.inline_bytes, "inline_bytes")?,
        raw_bytes: stored(figures.raw_bytes, "raw_bytes")?,
        bucket: figures.bucket,
    })
}
