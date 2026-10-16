//! The access-log writer: requests queue their records without waiting, and one task appends whatever is queued as
//! one batch — so many requests make one transaction rather than one each. A full queue turns a record away and
//! counts it; the request is served either way. On stop the writer lands what is queued, then lets go of the store.

use std::sync::Arc;
use std::time::Duration;

use tessari_s3_constants::observability::{
    AUDIT_APPEND_ATTEMPTS, AUDIT_BATCH_BYTES, AUDIT_BATCH_ROWS, AUDIT_RETRY_MS,
};
use tessari_s3_observability::Metrics;
use tessari_s3_storage::audit::{AccessRecord, AuditService};
use tokio::sync::{mpsc, watch};

/// The task that appends queued access-log records.
pub struct AuditWriter {
    receiver: mpsc::Receiver<AccessRecord>,
    audit: AuditService,
    metrics: Arc<Metrics>,
}

/// About how many bytes `record` takes in a batch.
fn estimate(record: &AccessRecord) -> usize {
    let texts = [
        Some(record.request_id.as_str()),
        Some(record.method.as_str()),
        Some(record.principal.as_str()),
        record.bucket.as_deref(),
        record.key.as_deref(),
        record.client.as_deref(),
    ];
    texts
        .iter()
        .flatten()
        .map(|text| text.len())
        .fold(128, usize::saturating_add)
}

impl AuditWriter {
    pub(crate) const fn new(
        receiver: mpsc::Receiver<AccessRecord>,
        audit: AuditService,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            receiver,
            audit,
            metrics,
        }
    }

    /// Appends queued records until `stop` changes, then lands the queue.
    pub async fn run(mut self, mut stop: watch::Receiver<bool>) {
        loop {
            let first = tokio::select! {
                biased;
                _ = stop.changed() => break,
                next = self.receiver.recv() => match next {
                    Some(record) => record,
                    None => break,
                },
            };
            let batch = self.gather(first);
            self.write(&batch).await;
        }
        self.receiver.close();
        while let Some(first) = self.receiver.recv().await {
            let batch = self.gather(first);
            self.write(&batch).await;
        }
        tracing::info!("access-log writer stopped; its queue is landed");
    }

    /// `first` and whatever else is queued right now, up to the row and byte bounds — checked before each receive,
    /// so a record taken from the queue is never left out.
    fn gather(&mut self, first: AccessRecord) -> Vec<AccessRecord> {
        let mut bytes = estimate(&first);
        let mut batch = vec![first];
        while batch.len() < AUDIT_BATCH_ROWS && bytes < AUDIT_BATCH_BYTES {
            match self.receiver.try_recv() {
                Ok(record) => {
                    bytes = bytes.saturating_add(estimate(&record));
                    batch.push(record);
                }
                Err(_) => break,
            }
        }
        batch
    }

    /// Appends `batch`, trying again after a failure; counts the records written or, at the last attempt, failed.
    async fn write(&self, batch: &[AccessRecord]) {
        let records = u64::try_from(batch.len()).unwrap_or(u64::MAX);
        let mut wait = Duration::from_millis(AUDIT_RETRY_MS);
        for attempt in 1..=AUDIT_APPEND_ATTEMPTS {
            match self.audit.append(batch).await {
                Ok(_) => {
                    self.metrics.audit_written(records);
                    return;
                }
                Err(error) if attempt < AUDIT_APPEND_ATTEMPTS => {
                    tracing::warn!(attempt, records, error = %error, "an access-log append failed; trying again");
                    tokio::time::sleep(wait).await;
                    wait = wait.saturating_mul(2);
                }
                Err(error) => {
                    tracing::error!(records, error = %error, "access-log records could not be appended");
                }
            }
        }
        self.metrics.audit_failed(records);
    }
}
