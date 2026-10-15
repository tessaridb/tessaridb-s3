//! Converting a store written before ADR-0008: every object record still holding its bytes gets a data file and loses
//! the bytes, then the column goes. Run at start, before the node serves; idempotent and resumable — a pass cut short
//! leaves records still holding bytes (converted by the next pass) and a queued data file for the reclaimer.

use tessari_s3_constants::{INLINE_CONVERT_BATCH, VERSION_CONVERT_BATCH};

use super::ObjectService;
use crate::Result;

impl ObjectService {
    /// Converts every record still holding object bytes and drops the `inline` column; answers how many were
    /// converted.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or the drive's failure. A record that keeps its bytes through a
    /// whole batch makes the column drop refuse, and the refusal is answered.
    pub async fn convert_inline(&self) -> Result<usize> {
        let mut converted: usize = 0;
        loop {
            let batch = self.repository.inline_records(INLINE_CONVERT_BATCH).await?;
            if batch.is_empty() {
                break;
            }
            let mut adopted = 0_usize;
            for record in batch {
                let size = u64::try_from(record.bytes.len()).unwrap_or(u64::MAX);
                let mut upload = self.upload(size).await?;
                let id = upload.id();
                let written = match upload.append(&record.bytes).await {
                    Ok(()) => upload.finish().await,
                    Err(error) => {
                        drop(upload);
                        Err(error)
                    }
                };
                let taken = match written {
                    Ok(uploaded) => {
                        self.repository
                            .adopt_data(&record.id, record.incarnation, uploaded.id)
                            .await
                    }
                    Err(error) => Err(error),
                };
                match taken {
                    Ok(true) => adopted = adopted.saturating_add(1),
                    Ok(false) => self.release_logged(id).await,
                    Err(error) => {
                        self.release_logged(id).await;
                        return Err(error);
                    }
                }
            }
            converted = converted.saturating_add(adopted);
            // A batch in which nothing could be converted would come back unchanged; the drop below names the record.
            if adopted == 0 {
                break;
            }
        }
        if self.repository.declares_inline().await? {
            self.repository.drop_inline().await?;
        }
        Ok(converted)
    }

    /// Gives every object a build without versions wrote its null version (ADR-0010 §8), once per store; answers how
    /// many were converted. Run at start, before the node serves: the event releasing a replaced object's parts goes
    /// first, then the walk, then the record saying it is done — a start cut short walks again, and an object already
    /// converted, or written since by a versioned build, is skipped by the conversion's own condition.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn convert_versions(&self) -> Result<usize> {
        if self.repository.versions_converted().await? {
            return Ok(0);
        }
        self.repository.drop_parts_released().await?;
        let mut converted: usize = 0;
        let mut after = String::new();
        loop {
            let batch = self
                .repository
                .objects_after(&after, VERSION_CONVERT_BATCH)
                .await?;
            let Some(last) = batch.last() else {
                break;
            };
            after.clone_from(&last.id);
            for object in &batch {
                if self.repository.adopt_null_version(object).await? {
                    converted = converted.saturating_add(1);
                }
            }
        }
        self.repository.mark_versions_converted().await?;
        Ok(converted)
    }
}
