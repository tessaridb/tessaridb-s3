//! The `lifecycle_runs` queue in TessariDB.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaError, MetaPool, RefusalClass, Value};

use super::entity::RunEntity;
use super::repository::LifecycleRepository;
use crate::{Error, Result};

/// The lifecycle queue over a metadata pool.
#[derive(Clone)]
pub struct TessariLifecycle {
    pool: MetaPool,
}

impl TessariLifecycle {
    /// The queue over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

/// A refusal that means the record moved or went: its compare-and-set condition, or a record no longer there.
fn moved_or_gone(error: &MetaError) -> bool {
    error.is_class(RefusalClass::Conflict) || error.is_class(RefusalClass::Invalid)
}

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "lifecycle_runs",
        reason,
    }
}

impl LifecycleRepository for TessariLifecycle {
    async fn claim(&self) -> Result<Option<RunEntity>> {
        let answers = self
            .pool
            .run("CLAIM FROM lifecycle_runs;", Vec::new())
            .await?;
        let record = match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => {
                records.into_iter().next().map(|(_, value)| value)
            }
            _ => None,
        };
        let Some(record) = record else {
            return Ok(None);
        };
        let Value::Object(fields) = record else {
            return Err(malformed("not an object"));
        };
        let bucket = match fields.get("name") {
            Some(Value::String(bucket)) => bucket.clone(),
            _ => return Err(malformed("bucket")),
        };
        let due = match fields.get("due") {
            Some(Value::Datetime { seconds, nanos }) => (*seconds, *nanos),
            _ => return Err(malformed("due")),
        };
        Ok(Some(RunEntity { bucket, due }))
    }

    async fn release(&self, bucket: &str, interval_secs: u64) -> Result<()> {
        let parameters = vec![
            ("bucket".to_owned(), Value::String(bucket.to_owned())),
            ("interval".to_owned(), crate::params::seconds(interval_secs)),
        ];
        let script = "RELEASE lifecycle_runs:$bucket NOT BEFORE duration::from_secs($interval);";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(()),
            // The rules were deleted while the bucket was worked: nothing is left to hand back.
            Err(error) if moved_or_gone(&error) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn forget(&self, run: &RunEntity) -> Result<bool> {
        let parameters = vec![
            ("bucket".to_owned(), Value::String(run.bucket.clone())),
            (
                "due".to_owned(),
                Value::Datetime {
                    seconds: run.due.0,
                    nanos: run.due.1,
                },
            ),
        ];
        // The condition is the claimed `due`: rules written meanwhile rewrote it, and their record must stay.
        let script = "BEGIN; UPDATE lifecycle_runs:$bucket SET name = $bucket WHERE due = $due; \
                      DELETE lifecycle_runs:$bucket; COMMIT;";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if moved_or_gone(&error) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}
