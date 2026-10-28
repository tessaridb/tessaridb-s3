//! [`LayoutMoveRepository`] over the metadata store: the `by_layout` index finds the ids left on an older layout, and
//! one transaction switches an id and records its retire.

use tessari_s3_infrastructure::tessaridb::{Answer, Number, RefusalClass, Value};

use super::moves_repository::LayoutMoveRepository;
use super::tessaridb::{TessariShardSets, data};
use crate::params::{count, seconds};
use crate::schema::DATA_HOLDERS;
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

impl TessariShardSets {
    /// Up to `limit` data ids on a layout older than `current`, those after `after` when it is given.
    async fn older_page(
        &self,
        current: u32,
        after: Option<[u8; 16]>,
        limit: usize,
    ) -> Result<Vec<[u8; 16]>> {
        let mut parameters = vec![
            (
                "current".to_owned(),
                Value::Number(Number::Integer(i64::from(current))),
            ),
            ("limit".to_owned(), count(limit)),
        ];
        let anchor = match after {
            Some(id) => {
                parameters.push(("after".to_owned(), Value::Uuid(id)));
                "AFTER shard_sets:$after "
            }
            None => "",
        };
        // The planner's estimate for this range can count the whole table (every id on the one layout there is), and
        // its scan guard then declines the index; the read is bounded by its limit, so walking the index is never the
        // costly path (Q-S3-94).
        let script = format!(
            "SELECT layout FROM shard_sets WHERE layout < $current {anchor}LIMIT $limit \
             WITHOUT SCAN GUARD USING INDEX by_layout;"
        );
        match self.pool.run(&script, parameters).await?.into_iter().next() {
            Some(Answer::Records { records, .. }) => {
                records.iter().map(|(key, _)| spelled(key)).collect()
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Which of `ids` are data nothing holds: queued for reclaim, and no object, version or part names them — the test
    /// the reclaimer makes before it removes anything. Queued alone is not enough: superseded data a version still
    /// holds is queued too, until the reclaimer finds it held and takes it off the queue.
    async fn unheld(&self, ids: &[[u8; 16]]) -> Result<Vec<bool>> {
        let script: String = (0..ids.len())
            .map(|index| format!("SELECT data FROM ONLY reclaims:$d{index};"))
            .collect();
        let parameters = ids
            .iter()
            .enumerate()
            .map(|(index, id)| (format!("d{index}"), Value::Uuid(*id)))
            .collect();
        let queued = self.pool.run(&script, parameters).await?;
        if queued.len() != ids.len() {
            return Err(malformed("an answer per data id"));
        }
        let mut unheld = Vec::with_capacity(ids.len());
        for (id, queued) in ids.iter().zip(&queued) {
            unheld.push(answered(queued) && !self.held(*id).await?);
        }
        Ok(unheld)
    }

    /// Whether anything holds data `id`.
    async fn held(&self, id: [u8; 16]) -> Result<bool> {
        let answers = self.pool.run(DATA_HOLDERS, data(id)).await?;
        Ok(answers.iter().any(answered))
    }
}

/// Whether `answer` carries a record.
fn answered(answer: &Answer) -> bool {
    matches!(answer, Answer::Records { records, .. } if !records.is_empty())
}

impl LayoutMoveRepository for TessariShardSets {
    async fn older(&self, current: u32, limit: usize) -> Result<Vec<[u8; 16]>> {
        // A shard set is recorded, and its id queued for reclaim, before the first shard is written; an id still
        // queued that nothing holds is not data — a write refused below its quorum, cut short, or not committed yet
        // — and is neither moved nor counted: it has nothing a move could read, and the reclaim removes it
        // (Q-S3-107). The index is walked page by page past such ids, so they never take the place of data behind
        // them.
        let mut found = Vec::new();
        let mut after = None;
        while found.len() < limit {
            let page = self.older_page(current, after, limit).await?;
            let Some(&last) = page.last() else {
                break;
            };
            let unheld = self.unheld(&page).await?;
            let full = page.len() == limit;
            found.extend(
                page.into_iter()
                    .zip(unheld)
                    .filter(|(_, unheld)| !unheld)
                    .map(|(id, _)| id),
            );
            if !full {
                break;
            }
            after = Some(last);
        }
        found.truncate(limit);
        Ok(found)
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
