//! [`ShardSetRepository`] over TessariDB. Every value is bound; the pool sends the namespace and database with each
//! unit of work.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Number, Value};

use super::repository::ShardSetRepository;
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
    async fn queue(&self, id: [u8; 16], layout: u32) -> Result<()> {
        let mut parameters = data(id);
        parameters.push(("layout".to_owned(), Value::from(i64::from(layout))));
        // One transaction: an id is never queued without its layout, nor recorded without being queued.
        let script = "BEGIN; UPSERT gc:$data SET data = $data, queued = time::now(); \
                      CREATE shard_sets:$data = { layout: $layout }; COMMIT;";
        self.pool.run(script, parameters).await?;
        Ok(())
    }

    async fn layout_of(&self, id: [u8; 16]) -> Result<Option<u32>> {
        let answers = self
            .pool
            .run("SELECT layout FROM ONLY shard_sets:$data;", data(id))
            .await?;
        let malformed = Error::Malformed {
            record: "shard_sets",
            reason: "layout",
        };
        match first_record(answers.into_iter().next())? {
            None => Ok(None),
            Some(Value::Object(fields)) => match fields.get("layout") {
                Some(Value::Number(Number::Integer(layout))) => {
                    u32::try_from(*layout).map(Some).map_err(|_| malformed)
                }
                _ => Err(malformed),
            },
            Some(_) => Err(malformed),
        }
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
}
