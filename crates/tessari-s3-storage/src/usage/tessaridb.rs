//! Usage in TessariDB: one grouped read over `objects` measures every bucket, the result replaces the one
//! `bucket_usage:1` record, and the `usage_claims` space holds the expiring claim of the member measuring now. Every
//! value is bound.

use tessari_s3_constants::USAGE_CLAIM_SECS;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use super::entity::{BucketUsageEntity, UsageEntity};
use super::repository::UsageRepository;
use crate::answers::first_record;
use crate::{Error, Result};

/// The TessariDB usage repository.
#[derive(Clone)]
pub struct TessariUsage {
    pool: MetaPool,
}

impl TessariUsage {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket_usage",
        reason,
    }
}

impl UsageRepository for TessariUsage {
    async fn claim(&self, holder: &str) -> Result<bool> {
        let parameters = vec![
            ("holder".to_owned(), Value::String(holder.to_owned())),
            ("pass".to_owned(), Value::String("usage".to_owned())),
        ];
        // Free, or already this member's. `EXPIRE` takes the duration as written; it is this server's own constant,
        // formatted to digits only.
        let script = format!(
            "SET usage_claims:$pass = $holder IF ABSENT EXPIRE {USAGE_CLAIM_SECS}s; \
             SET usage_claims:$pass = $holder IF = $holder EXPIRE {USAGE_CLAIM_SECS}s;"
        );
        let mut taken = false;
        for answer in self.pool.run(&script, parameters).await? {
            match answer {
                Answer::Value {
                    value: Value::Bool(wrote),
                    ..
                } => taken = taken || wrote,
                _ => return Err(malformed("a conditional set answered no boolean")),
            }
        }
        Ok(taken)
    }

    async fn measure(&self) -> Result<Vec<BucketUsageEntity>> {
        // A whole-table fold: it reads every object record by design, once per pass, never per request.
        let answers = self
            .pool
            .run(
                "SELECT bucket_name, count(*) AS objects, sum(size) AS bytes FROM objects GROUP BY bucket_name;",
                Vec::new(),
            )
            .await?;
        match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => records
                .iter()
                .map(|(_, value)| BucketUsageEntity::from_value(value, "bucket_name"))
                .collect(),
            _ => Err(malformed("a grouped read answered no records")),
        }
    }

    async fn store(&self, buckets: &[BucketUsageEntity]) -> Result<()> {
        let parameters = vec![(
            "buckets".to_owned(),
            Value::Array(buckets.iter().map(BucketUsageEntity::to_value).collect()),
        )];
        // The whole record, deliberately: a bucket emptied since the last pass must disappear from it.
        self.pool
            .run(
                "UPSERT bucket_usage:1 = { taken: time::now(), buckets: $buckets };",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn latest(&self) -> Result<Option<UsageEntity>> {
        let answers = self
            .pool
            .run("SELECT * FROM ONLY bucket_usage:1;", Vec::new())
            .await?;
        first_record(answers.into_iter().next())?
            .as_ref()
            .map(UsageEntity::from_value)
            .transpose()
    }
}
