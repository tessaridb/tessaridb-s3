//! The reaper's pass: uploads open longer than the maximum age are aborted — S3 counts an incomplete upload's age from
//! its initiation — each in the same one-transaction abort a client's AbortMultipartUpload runs, so the listing row
//! and every part's file go exactly as they would then.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tessari_s3_types::Timestamp;

use super::model::Reaped;
use super::repository::MultipartRepository;
use super::service::MultipartService;
use crate::{Error, Result};

impl MultipartService {
    /// Aborts at most `limit` uploads initiated more than `max_age_secs` ago.
    ///
    /// # Errors
    /// The metadata store's refusal or outage; a clock outside the representable range.
    pub async fn reap(&self, max_age_secs: u64, limit: usize) -> Result<Reaped> {
        let clock = || Error::Malformed {
            record: "clock",
            reason: "the system clock is outside the representable range",
        };
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(max_age_secs))
            .and_then(|instant| instant.duration_since(UNIX_EPOCH).ok())
            .ok_or_else(clock)?;
        let cutoff = Timestamp {
            seconds: i64::try_from(cutoff.as_secs()).map_err(|_| clock())?,
            nanos: cutoff.subsec_nanos(),
        };
        let due = self.repository.initiated_before(cutoff, limit).await?;
        let mut reaped = Reaped {
            examined: due.len(),
            ..Reaped::default()
        };
        for id in due {
            self.repository.abort(id).await?;
            reaped.aborted = reaped.aborted.saturating_add(1);
        }
        Ok(reaped)
    }
}
