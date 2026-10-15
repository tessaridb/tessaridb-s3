//! Failovers in TessariDB: `region_failovers:<region>` in the global database, never deleted; every value is bound,
//! and a `WHERE` on an edit is a compare-and-set, so a refused condition writes nothing.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Number, RefusalClass, Value};

use super::entity::RegionEntity;
use super::repository::{Change, RegionRepository};
use crate::Result;
use crate::answers::{all_records, first_record};

/// The TessariDB failover repository.
#[derive(Clone)]
pub struct TessariRegions {
    /// The global database, which every region reads.
    pool: MetaPool,
}

impl TessariRegions {
    /// A repository over the global `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn string(name: &str, value: &str) -> (String, Value) {
    (name.to_owned(), Value::String(value.to_owned()))
}

fn parameters(change: &Change<'_>) -> Vec<(String, Value)> {
    vec![
        string("region", change.region),
        (
            "epoch".to_owned(),
            Value::Number(Number::Integer(change.epoch)),
        ),
        (
            "promoted".to_owned(),
            change
                .promoted
                .map_or(Value::None, |region| Value::String(region.to_owned())),
        ),
        string("reason", change.reason),
        string("by", change.by),
    ]
}

/// `Ok(true)` for a write that landed, `Ok(false)` for one that lost: its condition was false (`conflict`), or another
/// change to the record committed after this one's snapshot (`retry`) — either way the record moved and nothing here was
/// written.
fn landed<T>(answer: tessari_s3_infrastructure::tessaridb::MetaResult<T>) -> Result<bool> {
    match answer {
        Ok(_) => Ok(true),
        Err(error)
            if error.is_class(RefusalClass::Conflict) || error.is_class(RefusalClass::Retry) =>
        {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

impl RegionRepository for TessariRegions {
    async fn get(&self, region: &str) -> Result<Option<RegionEntity>> {
        let answers = self
            .pool
            .run(
                "SELECT * FROM ONLY region_failovers:$region;",
                vec![string("region", region)],
            )
            .await?;
        first_record(answers.into_iter().next())?
            .map(|value| RegionEntity::from_value(&value))
            .transpose()
    }

    async fn list(&self) -> Result<Vec<RegionEntity>> {
        // 256: far more regions than a deployment names.
        let answers = self
            .pool
            .run(
                "SELECT * FROM region_failovers ORDER BY region LIMIT 256;",
                Vec::new(),
            )
            .await?;
        all_records(answers.into_iter().next(), "region failover")?
            .iter()
            .map(RegionEntity::from_value)
            .collect()
    }

    async fn create(&self, change: &Change<'_>) -> Result<bool> {
        let script = "CREATE region_failovers:$region = { region: $region, epoch: $epoch, promoted: $promoted, \
                      reason: $reason, actor: $by, at: time::now() };";
        landed(self.pool.run(script, parameters(change)).await)
    }

    async fn replace(&self, change: &Change<'_>, seen: i64) -> Result<bool> {
        let script = "UPDATE region_failovers:$region SET epoch = $epoch, promoted = $promoted, \
                      reason = $reason, actor = $by, at = time::now() WHERE epoch = $seen;";
        let mut parameters = parameters(change);
        parameters.push(("seen".to_owned(), Value::Number(Number::Integer(seen))));
        landed(self.pool.run(script, parameters).await)
    }
}
