//! [`LayoutMoveRepository`] over the metadata store: the `by_layout` index finds the ids left on an older layout, and
//! one transaction switches an id and records its retire.

use tessari_s3_infrastructure::tessaridb::{Answer, Number, RefusalClass, Value};

use super::moves_repository::LayoutMoveRepository;
use super::tessaridb::{TessariShardSets, data};
use crate::params::{count, seconds};
use crate::{Error, Result};

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "shard_sets",
        reason,
    }
}

/// A data id as a record answer spells it: `uuid '…'`.
fn spelled(key: &str) -> Result<[u8; 16]> {
    let inner = key
        .strip_prefix("uuid '")
        .and_then(|rest| rest.strip_suffix('\''))
        .ok_or(malformed("a data id that is not a uuid"))?;
    uuid::Uuid::parse_str(inner)
        .map(|id| id.into_bytes())
        .map_err(|_| malformed("a data id that is not a uuid"))
}

impl LayoutMoveRepository for TessariShardSets {
    async fn older(&self, current: u32, limit: usize) -> Result<Vec<[u8; 16]>> {
        let parameters = vec![
            (
                "current".to_owned(),
                Value::Number(Number::Integer(i64::from(current))),
            ),
            ("limit".to_owned(), count(limit)),
        ];
        let answers = self
            .pool
            .run(
                "SELECT layout FROM shard_sets WHERE layout < $current LIMIT $limit USING INDEX by_layout;",
                parameters,
            )
            .await?;
        match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => {
                records.iter().map(|(key, _)| spelled(key)).collect()
            }
            _ => Ok(Vec::new()),
        }
    }

    async fn switch(&self, id: [u8; 16], old: u32, new: u32) -> Result<bool> {
        let mut parameters = data(id);
        parameters.push((
            "old".to_owned(),
            Value::Number(Number::Integer(i64::from(old))),
        ));
        parameters.push((
            "new".to_owned(),
            Value::Number(Number::Integer(i64::from(new))),
        ));
        let script = "BEGIN; UPDATE shard_sets:$data SET layout = $new WHERE layout = $old; \
                      CREATE layout_retire:$data = { old: $old, switched: time::now() }; COMMIT;";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn due_retires(&self, grace_secs: u64, limit: usize) -> Result<Vec<([u8; 16], u32)>> {
        let parameters = vec![
            ("grace".to_owned(), seconds(grace_secs)),
            ("limit".to_owned(), count(limit)),
        ];
        let answers = self
            .pool
            .run(
                "SELECT old FROM layout_retire WHERE switched <= time::now() - duration::from_secs($grace) \
                 LIMIT $limit USING INDEX by_switched;",
                parameters,
            )
            .await?;
        let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
            return Ok(Vec::new());
        };
        records
            .iter()
            .map(|(key, record)| {
                let Value::Object(fields) = record else {
                    return Err(malformed("a retire that is not an object"));
                };
                let Some(Value::Number(Number::Integer(old))) = fields.get("old") else {
                    return Err(malformed("a retire without its layout"));
                };
                let old = u32::try_from(*old).map_err(|_| malformed("a retire's layout"))?;
                Ok((spelled(key)?, old))
            })
            .collect()
    }

    async fn retired(&self, id: [u8; 16]) -> Result<()> {
        self.pool
            .run("DELETE layout_retire:$data;", data(id))
            .await?;
        Ok(())
    }
}
