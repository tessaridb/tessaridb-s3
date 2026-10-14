//! [`ShardSetRepository`] over TessariDB. Every value is bound; the pool sends the namespace and database with each
//! unit of work.

use tessari_s3_constants::{HEAL_CLAIM_SECS, HEAL_RETRY_SECS};
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};

use super::repository::{ShardSet, ShardSetRepository};
use crate::answers::first_record;
use crate::schema::QUEUE_FOR_RECLAIM;
use crate::{Error, Result};

/// Shard-set records over a metadata pool.
#[derive(Clone)]
pub(crate) struct TessariShardSets {
    pool: MetaPool,
}

impl TessariShardSets {
    pub(crate) const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn data(id: [u8; 16]) -> Vec<(String, Value)> {
    vec![("data".to_owned(), Value::Uuid(id))]
}

impl ShardSetRepository for TessariShardSets {
    async fn queue(&self, id: [u8; 16], set: ShardSet) -> Result<()> {
        let size = i64::try_from(set.size).map_err(|_| Error::Malformed {
            record: "shard_sets",
            reason: "a size beyond i64",
        })?;
        let mut parameters = data(id);
        parameters.push(("layout".to_owned(), Value::from(i64::from(set.layout))));
        parameters.push(("size".to_owned(), Value::from(size)));
        // One transaction: an id is never queued without its shard set, nor recorded without being queued.
        let script = format!(
            "BEGIN; {QUEUE_FOR_RECLAIM} CREATE shard_sets:$data = {{ layout: $layout, size: $size }}; COMMIT;"
        );
        self.pool.run(&script, parameters).await?;
        Ok(())
    }

    async fn shard_set(&self, id: [u8; 16]) -> Result<Option<ShardSet>> {
        let answers = self
            .pool
            .run("SELECT layout, size FROM ONLY shard_sets:$data;", data(id))
            .await?;
        let malformed = |reason| Error::Malformed {
            record: "shard_sets",
            reason,
        };
        let Some(record) = first_record(answers.into_iter().next())? else {
            return Ok(None);
        };
        let Value::Object(fields) = record else {
            return Err(malformed("not an object"));
        };
        let integer = |name: &str| match fields.get(name) {
            Some(Value::Number(Number::Integer(value))) => Some(*value),
            _ => None,
        };
        let layout = integer("layout")
            .and_then(|layout| u32::try_from(layout).ok())
            .ok_or_else(|| malformed("layout"))?;
        let size = integer("size")
            .and_then(|size| u64::try_from(size).ok())
            .ok_or_else(|| malformed("size"))?;
        Ok(Some(ShardSet { layout, size }))
    }

    async fn forget(&self, id: [u8; 16]) -> Result<()> {
        self.pool.run("DELETE shard_sets:$data;", data(id)).await?;
        Ok(())
    }

    async fn heal(&self, id: [u8; 16]) -> Result<()> {
        // A listing of an id already listed keeps its attempt count; `due` now puts it back in line.
        self.pool
            .run(
                "UPSERT heal_queue:$data SET data = $data, due = time::now();",
                data(id),
            )
            .await?;
        Ok(())
    }

    async fn take_heal(&self) -> Result<Option<[u8; 16]>> {
        let answers = self.pool.run("CLAIM FROM heal_queue;", Vec::new()).await?;
        Ok(heal_records(answers)?.into_iter().next().map(|(id, _)| id))
    }

    async fn retry_heal(&self, id: [u8; 16]) -> Result<()> {
        let mut parameters = data(id);
        parameters.push(("retry".to_owned(), crate::params::seconds(HEAL_RETRY_SECS)));
        self.pool
            .run(
                "RELEASE heal_queue:$data NOT BEFORE duration::from_secs($retry);",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn listed(&self, limit: usize) -> Result<Vec<([u8; 16], i64)>> {
        let parameters = vec![("limit".to_owned(), crate::params::count(limit))];
        heal_records(
            self.pool
                .run(
                    "SELECT data, attempts FROM heal_queue LIMIT $limit;",
                    parameters,
                )
                .await?,
        )
    }

    async fn healed(&self, id: [u8; 16]) -> Result<()> {
        self.pool.run("DELETE heal_queue:$data;", data(id)).await?;
        Ok(())
    }

    async fn claim(&self, id: [u8; 16], holder: &str) -> Result<bool> {
        let mut parameters = data(id);
        parameters.push(("holder".to_owned(), Value::String(holder.to_owned())));
        // Free, or already this holder's — one healer and one reclaimer run per process, one after the other, so a
        // claim under this node's own name is one a dead process of this node left. `EXPIRE` takes the duration as
        // written; it is this server's own constant, formatted to digits only.
        let script = format!(
            "SET heal_claims:$data = $holder IF ABSENT EXPIRE {HEAL_CLAIM_SECS}s; \
             SET heal_claims:$data = $holder IF = $holder EXPIRE {HEAL_CLAIM_SECS}s;"
        );
        let mut taken = false;
        for answer in self.pool.run(&script, parameters).await? {
            match answer {
                Answer::Value {
                    value: Value::Bool(wrote),
                    ..
                } => taken = taken || wrote,
                _ => {
                    return Err(Error::Malformed {
                        record: "heal_claims",
                        reason: "a conditional set answered no boolean",
                    });
                }
            }
        }
        Ok(taken)
    }

    async fn unclaim(&self, id: [u8; 16], holder: &str) -> Result<()> {
        let mut parameters = data(id);
        parameters.push(("holder".to_owned(), Value::String(holder.to_owned())));
        // Conditional on still holding it, and leaving an expiring key: a bare write would make the claim permanent.
        self.pool
            .run(
                "SET heal_claims:$data = 'free' IF = $holder EXPIRE 1ms;",
                parameters,
            )
            .await?;
        Ok(())
    }
}

/// The data ids of an answer over `heal_queue`, each with its hand-out count (none before the first claim).
fn heal_records(answers: Vec<Answer>) -> Result<Vec<([u8; 16], i64)>> {
    let malformed = |reason| Error::Malformed {
        record: "heal_queue",
        reason,
    };
    let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
        return Ok(Vec::new());
    };
    records
        .into_iter()
        .map(|(_, record)| {
            let Value::Object(fields) = record else {
                return Err(malformed("not an object"));
            };
            let Some(Value::Uuid(id)) = fields.get("data") else {
                return Err(malformed("data"));
            };
            let attempts = match fields.get("attempts") {
                Some(Value::Number(Number::Integer(attempts))) => *attempts,
                _ => 0,
            };
            Ok((*id, attempts))
        })
        .collect()
}
