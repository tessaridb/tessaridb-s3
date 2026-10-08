//! Moves the work lists an earlier build kept on plain tables onto the queues of ADR-0009, at start-up. The events that
//! queued replaced data into `gc` are dropped and the schema applied again, which defines them anew (a definition
//! saying `IF NOT EXISTS` never replaces one); then each `gc` entry becomes a `reclaims` record held back until its queuing instant plus the recorded grace, with its
//! `reclaiming` mark kept, and each `heals` entry a `heal_queue` record due at once. Every entry moves in its own
//! transaction together with its removal, so a start cut short resumes where it stopped; an emptied table is dropped.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::{Error, Result};

/// Entries one read takes from an old table.
const BATCH: usize = 100;

/// Converts the old work lists still present; answers how many entries moved.
///
/// # Errors
/// The metadata store's refusal or outage.
pub(crate) async fn convert(pool: &MetaPool, reclaim_grace_secs: u64) -> Result<usize> {
    let mut stale = Vec::new();
    for (event, table) in [("supersede", "objects"), ("part_superseded", "parts")] {
        if queues_into_gc(pool, table).await? {
            stale.push(format!("DROP EVENT {event} ON {table};"));
        }
    }
    if !stale.is_empty() {
        // One transaction: both go, or neither. A start stopped before the schema is applied again finds them gone and
        // its own schema application, which runs first, defines them.
        pool.run(&format!("BEGIN; {} COMMIT;", stale.join(" ")), Vec::new())
            .await?;
        crate::schema::apply(pool, reclaim_grace_secs).await?;
    }
    let tables = tables(pool).await?;
    let mut moved = 0_usize;
    if tables.iter().any(|name| name == "gc") {
        moved = moved.saturating_add(
            drain(
                pool,
                "gc",
                "SELECT data, queued, reclaiming FROM gc",
                "BEGIN; LET $settings = (SELECT grace FROM ONLY s3_settings:'reclaim'); \
                 LET $grace = $settings.grace; \
                 UPSERT reclaims:$data SET data = $data, \
                 due = time::from_unix(time::unix($queued) + $grace), reclaiming = $reclaiming; \
                 DELETE gc:$data; COMMIT;",
            )
            .await?,
        );
    }
    if tables.iter().any(|name| name == "heals") {
        moved = moved.saturating_add(
            drain(
                pool,
                "heals",
                "SELECT data, queued FROM heals",
                "BEGIN; UPSERT heal_queue:$data SET data = $data, due = $queued; DELETE heals:$data; COMMIT;",
            )
            .await?,
        );
    }
    Ok(moved)
}

/// Whether `table` (this module's own literal) still carries an event queuing into `gc`, as its definition reads.
async fn queues_into_gc(pool: &MetaPool, table: &str) -> Result<bool> {
    match pool
        .run(&format!("INFO FOR TABLE {table};"), Vec::new())
        .await?
        .into_iter()
        .next()
    {
        Some(Answer::Value {
            value: Value::Object(info),
            ..
        }) => match info.get("definition") {
            Some(Value::String(definition)) => Ok(definition.contains("UPSERT gc:")),
            _ => Err(Error::Malformed {
                record: "table info",
                reason: "definition",
            }),
        },
        _ => Err(Error::Malformed {
            record: "table info",
            reason: "not a value",
        }),
    }
}

/// The tables of the configured database.
async fn tables(pool: &MetaPool) -> Result<Vec<String>> {
    let malformed = |reason| Error::Malformed {
        record: "database info",
        reason,
    };
    match pool
        .run("INFO FOR DATABASE;", Vec::new())
        .await?
        .into_iter()
        .next()
    {
        Some(Answer::Value {
            value: Value::Object(info),
            ..
        }) => match info.get("tables") {
            Some(Value::Array(names)) => names
                .iter()
                .map(|name| match name {
                    Value::String(name) => Ok(name.clone()),
                    _ => Err(malformed("a table name")),
                })
                .collect(),
            _ => Err(malformed("tables")),
        },
        _ => Err(malformed("not a value")),
    }
}

/// Moves every entry `read` finds in `table` with `step` (bound `$data`, `$queued`, `$reclaiming`), then drops the
/// emptied table; `table` and `read` are this module's own literals, never a caller's value.
async fn drain(pool: &MetaPool, table: &str, read: &str, step: &str) -> Result<usize> {
    let malformed = |reason| Error::Malformed {
        record: "work list",
        reason,
    };
    let mut moved = 0_usize;
    loop {
        let answers = pool
            .run(&format!("{read} LIMIT {BATCH};"), Vec::new())
            .await?;
        let records = match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => records,
            _ => Vec::new(),
        };
        if records.is_empty() {
            break;
        }
        for (_, record) in records {
            let Value::Object(fields) = record else {
                return Err(malformed("not an object"));
            };
            let data = fields
                .get("data")
                .cloned()
                .ok_or_else(|| malformed("data"))?;
            let queued = fields
                .get("queued")
                .cloned()
                .ok_or_else(|| malformed("queued"))?;
            let reclaiming = fields.get("reclaiming").cloned().unwrap_or(Value::None);
            let parameters = vec![
                ("data".to_owned(), data),
                ("queued".to_owned(), queued),
                ("reclaiming".to_owned(), reclaiming),
            ];
            pool.run(step, parameters).await?;
            moved = moved.saturating_add(1);
        }
    }
    pool.run(&format!("DROP TABLE {table};"), Vec::new())
        .await?;
    Ok(moved)
}
