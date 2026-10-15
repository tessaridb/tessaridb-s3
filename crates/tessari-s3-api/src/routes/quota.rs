//! Bucket quotas on the write path (ADR-0007): an operation that adds to a bucket (`quota::ADDING`) asks for room
//! before it does the work. A bucket with no limit costs one bucket read; one with a limit also reads the last usage
//! measurement, so the stated bound is what is written within one usage interval. A measurement older than
//! `QUOTA_MEASUREMENT_INTERVALS` usage intervals (or none at all) is not trusted: the write counts that one bucket
//! instead, within `QUOTA_COUNT_TIMEOUT_SECS`, and a count that cannot answer refuses the write as unavailable. A store
//! that cannot answer refuses it too — the quota fails closed.

use tessari_s3_constants::{QUOTA_COUNT_TIMEOUT_SECS, QUOTA_MEASUREMENT_INTERVALS};
use tessari_s3_core::quota::{Adding, Held, Quota, admits, measured_recently};
use tessari_s3_storage::usage::BucketCount;
use tessari_s3_types::{BucketName, ErrorCode, ObjectKey};

use crate::pipeline::call::Call;
use crate::{Error, Result};

/// A limited bucket's limits and what it held when last measured (or counted, when that was too long ago).
pub(crate) struct Room {
    quota: Quota,
    held: Held,
}

impl Room {
    /// The room in `bucket`, or `None` when it has no limit (or does not exist — the write then answers that).
    ///
    /// # Errors
    /// `ServiceUnavailable` when a limited bucket has no recent enough measurement and could not be counted; the store's
    /// failure to read the bucket or the measurement.
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
        let held = match usage {
            // A bucket missing from the measurement held nothing when it was taken.
            Some(usage) => usage
                .buckets
                .into_iter()
                .find(|each| each.bucket == bucket.as_str())
                .map_or_else(Held::default, |each| Held {
                    objects: each.objects,
                    bytes: each.bytes,
                }),
            None => {
                tracing::warn!(
                    bucket = bucket.as_str(),
                    interval_secs = interval,
                    intervals = QUOTA_MEASUREMENT_INTERVALS,
                    "no usage measurement within the quota window; counting the bucket on the write path"
                );
                counted(bucket, storage.usage().count_bucket(bucket).await)?
            }
        };
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

/// What a bucket counted on the write path holds; a count that could not answer refuses the write as unavailable.
fn counted(
    bucket: &BucketName,
    count: std::result::Result<BucketCount, tessari_s3_storage::Error>,
) -> Result<Held> {
    count
        .map(|count| Held {
            objects: count.objects,
            bytes: count.bytes,
        })
        .map_err(|error| {
            tracing::warn!(
                bucket = bucket.as_str(),
                timeout_secs = QUOTA_COUNT_TIMEOUT_SECS,
                error = %error,
                "refused a write to a limited bucket: its usage could not be counted"
            );
            Error::new(
                ErrorCode::ServiceUnavailable,
                "the bucket's usage was not measured recently and could not be counted to check its quota; retry",
            )
        })
}

#[cfg(test)]
mod tests {
    use tessari_s3_core::quota::Held;
    use tessari_s3_storage::usage::BucketCount;
    use tessari_s3_types::{BucketName, ErrorCode};

    use super::counted;

    #[test]
    fn a_count_that_could_not_answer_refuses_the_write_as_unavailable() {
        let bucket = BucketName::new("held").expect("a name");
        let refused =
            counted(&bucket, Err(tessari_s3_storage::Error::NoLayout)).expect_err("refused");
        assert_eq!(refused.code, ErrorCode::ServiceUnavailable);
        let count = BucketCount {
            objects: 2,
            bytes: 9,
        };
        assert_eq!(
            counted(&bucket, Ok(count)).expect("counted"),
            Held {
                objects: 2,
                bytes: 9
            }
        );
    }
}
