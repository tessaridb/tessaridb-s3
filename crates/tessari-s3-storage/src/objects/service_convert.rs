//! Converting a store written before ADR-0008: every object record still holding its bytes gets a data file and loses
//! the bytes, then the column goes. Run at start, before the node serves; idempotent and resumable — a pass cut short
//! leaves records still holding bytes (converted by the next pass) and a queued data file for the reclaimer.

use tessari_s3_constants::INLINE_CONVERT_BATCH;

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
}
