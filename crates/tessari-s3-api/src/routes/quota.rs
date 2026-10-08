//! Bucket quotas on the write path (ADR-0007): an operation that adds to a bucket (`quota::ADDING`) asks for room
//! before it does the work. A bucket with no limit costs one bucket read; one with a limit also reads the last usage
//! measurement, so the stated bound is what is written within one usage interval. A store that cannot answer refuses
//! the write, and so does a measurement older than `QUOTA_MEASUREMENT_INTERVALS` usage intervals (or none at all) —
//! the quota fails closed rather than admitting writes on a figure nobody refreshed.

use tessari_s3_constants::QUOTA_MEASUREMENT_INTERVALS;
use tessari_s3_core::quota::{Adding, Held, Quota, admits, measured_recently};
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
    /// `ServiceUnavailable` when a limited bucket has no recent enough measurement; the store's failure to read the
    /// bucket or the measurement.
    pub(crate) async fn of(call: &Call<'_>, bucket: &BucketName) -> Result<Option<Self>> {
        let storage = call.state.storage();
        let Some(found) = storage.buckets().get(bucket).await? else {
            return Ok(None);
        };
        if !found.quota.limits() {
            return Ok(None);
        }
        let now = call.state.now();
        let interval = call.state.usage_interval_secs();
        let usage = storage
            .usage()
            .latest()
            .await?
            .filter(|usage| measured_recently(usage.taken.seconds, now, interval));
        let Some(usage) = usage else {
            tracing::warn!(
                bucket = bucket.as_str(),
                interval_secs = interval,
                intervals = QUOTA_MEASUREMENT_INTERVALS,
                "refused a write to a limited bucket: no usage measurement within the quota window"
            );
            return Err(Error::new(
                ErrorCode::ServiceUnavailable,
                "the bucket's usage was not measured recently enough to check its quota; retry once the usage \
                 scanner has run",
            ));
        };
        // A bucket missing from the measurement held nothing when it was taken.
        let held = usage
            .buckets
            .into_iter()
            .find(|each| each.bucket == bucket.as_str())
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
