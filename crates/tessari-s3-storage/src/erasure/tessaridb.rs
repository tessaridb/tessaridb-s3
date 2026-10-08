//! [`ShardSetRepository`] over TessariDB. Every value is bound; the pool sends the namespace and database with each
//! unit of work.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};

use super::repository::{ShardSet, ShardSetRepository};
use crate::answers::first_record;
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
        let script = "BEGIN; UPSERT gc:$data SET data = $data, queued = time::now(); \
                      CREATE shard_sets:$data = { layout: $layout, size: $size }; COMMIT;";
        self.pool.run(script, parameters).await?;
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
        self.pool
            .run(
                "UPSERT heals:$data SET data = $data, queued = time::now();",
                data(id),
            )
            .await?;
        Ok(())
    }

    async fn healing(&self, limit: usize) -> Result<Vec<[u8; 16]>> {
        // `LIMIT` takes a literal (0.33.1 refuses a parameter there); the limit is this server's own integer, never a
        // caller's value, and an integer formats to digits only.
        let script = format!("SELECT data FROM heals LIMIT {limit};");
        let answers = self.pool.run(&script, Vec::new()).await?;
        let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
            return Ok(Vec::new());
        };
        records
            .into_iter()
            .map(|(_, record)| match record {
                Value::Object(fields) => match fields.get("data") {
                    Some(Value::Uuid(id)) => Ok(*id),
                    _ => Err(Error::Malformed {
                        record: "heals",
                        reason: "data",
                    }),
                },
                _ => Err(Error::Malformed {
                    record: "heals",
                    reason: "not an object",
                }),
            })
            .collect()
    }

    async fn healed(&self, id: [u8; 16]) -> Result<()> {
        self.pool.run("DELETE heals:$data;", data(id)).await?;
        Ok(())
    }
}
