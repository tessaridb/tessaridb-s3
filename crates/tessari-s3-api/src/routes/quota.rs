//! Bucket quotas on the write path (ADR-0007): an operation that adds to a bucket (`quota::ADDING`) asks for room
//! before it does the work. A bucket with no limit costs one bucket read; one with a limit also reads the last usage
//! measurement, so the stated bound is what is written within one usage interval. A store that cannot answer refuses
//! the write — the quota fails closed.

use tessari_s3_core::quota::{Adding, Held, Quota, admits};
use tessari_s3_types::{BucketName, ErrorCode, ObjectKey};

use crate::pipeline::call::Call;
use crate::{Error, Result};

/// A limited bucket's limits and what it held when last measured.
pub(crate) struct Room {
    quota: Quota,
    held: Held,
}

impl Room {
    /// The room in `bucket`, or `None` when it has no limit (or does not exist — the write then answers that).
    ///
    /// # Errors
    /// The store's failure to read the bucket or the measurement.
    pub(crate) async fn of(call: &Call<'_>, bucket: &BucketName) -> Result<Option<Self>> {
        let storage = call.state.storage();
        let Some(found) = storage.buckets().get(bucket).await? else {
            return Ok(None);
        };
        if !found.quota.limits() {
            return Ok(None);
        }
        // A bucket missing from the measurement held nothing when it was taken.
        let held = storage
            .usage()
            .latest()
            .await?
            .and_then(|usage| {
                usage
                    .buckets
                    .into_iter()
                    .find(|each| each.bucket == bucket.as_str())
            })
            .map_or_else(Held::default, |each| Held {
                objects: each.objects,
                bytes: each.bytes,
            });
        Ok(Some(Self {
            quota: found.quota,
            held,
        }))
    }

    /// Refuses `bytes` more, and one object more when `key` would be new (asked only under an object limit).
    ///
    /// # Errors
    /// `InvalidRequest` when a limit would be passed; the store's failure to look the key up.
    pub(crate) async fn admit(
        &self,
        call: &Call<'_>,
        bucket: &BucketName,
        key: Option<&ObjectKey>,
        bytes: u64,
    ) -> Result<()> {
        let object = match key {
            Some(key) if self.quota.max_objects.is_some() => matches!(
                call.state.storage().objects().get(bucket, key).await?,
                Ok(None)
            ),
            _ => false,
        };
        admits(self.quota, self.held, Adding { bytes, object })
            .map_err(|_| Error::new(ErrorCode::InvalidRequest, "bucket quota exceeded"))
    }

    /// The most bytes a write may still add, under a byte limit.
    pub(crate) fn bytes_left(&self) -> Option<u64> {
        self.quota
            .max_bytes
            .map(|max| max.saturating_sub(self.held.bytes))
    }
}

/// Refuses a write of `bytes` to `bucket` (adding an object when `key` is new) that would pass its quota.
///
/// # Errors
/// As [`Room::of`] and [`Room::admit`].
pub(crate) async fn admit(
    call: &Call<'_>,
    bucket: &BucketName,
    key: Option<&ObjectKey>,
    bytes: u64,
) -> Result<()> {
    match Room::of(call, bucket).await? {
        Some(room) => room.admit(call, bucket, key, bytes).await,
        None => Ok(()),
    }
}
