//! The metadata schema, applied at start-up. Every definition says `IF NOT EXISTS`, so a node that restarts
//! re-runs it without effect.

use std::time::Duration;

use tessari_s3_constants::{SCHEMA_RETRY_ATTEMPTS, SCHEMA_RETRY_BACKOFF_MS};
use tessari_s3_infrastructure::tessaridb::{MetaPool, RefusalClass};

use crate::Result;

/// The table definitions, run inside the configured namespace and database. `gc` holds the data ids whose files may
/// be reclaimed: an upload's own id from before its file exists until the commit that references it, and — through
/// the `supersede` event, inside the very transaction that overwrites or deletes an object — the id it replaced.
/// Multipart uploads keep their own records (`uploads`, `parts`), never seen by an object read or a listing, and a
/// replaced or removed part queues its data the same way (`part_superseded`). A completed upload's parts become an
/// object's (`upload`, `parts`); when the object is overwritten or deleted, `parts_released` removes them, and each
/// removal queues its file. Each open upload also has a `pending` record whose id orders the uploads as
/// ListMultipartUploads lists them; whatever deletes the upload drops it in the same transaction (`upload_closed`).
/// The cluster's topology lives here too: `s3_nodes` (one record per node, refreshed as it registers) and `layouts`
/// (one record per layout version, created once and never rewritten). On a cluster member every data id is
/// erasure-coded and `shard_sets` records the layout its shards were placed under, from the transaction that queues
/// the id until reclamation; `heals` lists the data ids acknowledged with fewer than every shard durable, and the
/// `heal_claims` space holds the expiring claim of the one node healing or reclaiming an id.
/// The definitions commit as ONE transaction: a node never sees the tables without the event, and nodes starting
/// together contend once per attempt rather than once per definition.
const TABLES: &str = "\
BEGIN;
DEFINE TABLE IF NOT EXISTS buckets (\
 name string REQUIRED, created datetime REQUIRED, region string REQUIRED, incarnation uuid REQUIRED);
DEFINE TABLE IF NOT EXISTS objects (\
 bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED, size int REQUIRED, etag string REQUIRED,\
 modified datetime REQUIRED, headers object REQUIRED, metadata object REQUIRED, checksums object REQUIRED,\
 inline bytes, data uuid, upload uuid, parts int);
DEFINE TABLE IF NOT EXISTS gc (data uuid REQUIRED, queued datetime REQUIRED, reclaiming datetime);
DEFINE INDEX IF NOT EXISTS by_data ON objects FIELDS data;
DEFINE EVENT IF NOT EXISTS supersede ON objects FOR UPDATE, DELETE \
WHEN $before.data != NONE AND $before.data != $after.data \
THEN { LET $old = $before.data; UPSERT gc:$old SET data = $old, queued = time::now(); };
DEFINE TABLE IF NOT EXISTS uploads (\
 bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED, initiated datetime REQUIRED,\
 headers object REQUIRED, metadata object REQUIRED, checksum_algorithm string, checksum_type string,\
 last_part datetime, position string);
DEFINE TABLE IF NOT EXISTS parts (\
 upload uuid REQUIRED, number int REQUIRED, data uuid REQUIRED, size int REQUIRED, etag string REQUIRED,\
 modified datetime REQUIRED, checksums object REQUIRED);
DEFINE INDEX IF NOT EXISTS by_upload ON parts FIELDS upload;
DEFINE INDEX IF NOT EXISTS by_part_data ON parts FIELDS data;
DEFINE EVENT IF NOT EXISTS part_superseded ON parts FOR UPDATE, DELETE \
WHEN $before.data != NONE AND $before.data != $after.data \
THEN { LET $old = $before.data; UPSERT gc:$old SET data = $old, queued = time::now(); };
DEFINE EVENT IF NOT EXISTS parts_released ON objects FOR UPDATE, DELETE \
WHEN $before.upload != NONE AND $before.upload != $after.upload \
THEN { LET $old = $before.upload; DELETE FROM parts WHERE upload = $old LIMIT ALL; };
DEFINE TABLE IF NOT EXISTS pending (\
 upload uuid REQUIRED, bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED,\
 initiated datetime REQUIRED, position string REQUIRED);
DEFINE INDEX IF NOT EXISTS by_pending_initiated ON pending FIELDS initiated;
DEFINE INDEX IF NOT EXISTS by_pending_initiated ON pending FIELDS initiated;
DEFINE EVENT IF NOT EXISTS upload_closed ON uploads FOR DELETE WHEN $before.position != NONE \
THEN { LET $listed = $before.position; DELETE pending:$listed; };
DEFINE TABLE IF NOT EXISTS s3_nodes (node string REQUIRED, endpoint string REQUIRED, seen datetime REQUIRED);
DEFINE TABLE IF NOT EXISTS layouts (\
 version int REQUIRED, data int REQUIRED, parity int REQUIRED, nodes array REQUIRED, created datetime REQUIRED);
DEFINE TABLE IF NOT EXISTS shard_sets (layout int REQUIRED, size int REQUIRED);
DEFINE TABLE IF NOT EXISTS heals (data uuid REQUIRED, queued datetime REQUIRED);
DEFINE SPACE IF NOT EXISTS heal_claims;
COMMIT;
";

/// Creates the namespace, the database and the tables when they are missing. Several nodes starting together race
/// on the catalog and the store answers the loser `retry` rather than waiting, so a `retry` refusal is retried a
/// bounded number of times; every other refusal ends the attempt.
///
/// # Errors
/// The metadata store's refusal or outage, or the last `retry` refusal when every attempt lost.
pub async fn apply(pool: &MetaPool) -> Result<()> {
    let mut attempt: u32 = 1;
    loop {
        match apply_once(pool).await {
            Err(crate::Error::Meta(error))
                if error.is_class(RefusalClass::Retry) && attempt < SCHEMA_RETRY_ATTEMPTS =>
            {
                tracing::debug!(attempt, error = %error, "schema contended; retrying");
                let step = u64::from(attempt).saturating_mul(SCHEMA_RETRY_BACKOFF_MS);
                // Jitter, so nodes (or tests) that collided once do not retry in lockstep and collide again.
                let backoff = step.saturating_add(jitter(step));
                tokio::time::sleep(Duration::from_millis(backoff)).await;
                attempt = attempt.saturating_add(1);
            }
            outcome => return outcome,
        }
    }
}

/// A random value in `0..bound` from the standard library's randomly keyed hasher; no dependency needed for a
/// back-off spread.
fn jitter(bound: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(bound);
    hasher.finish().checked_rem(bound).unwrap_or(0)
}

async fn apply_once(pool: &MetaPool) -> Result<()> {
    // The names come from configuration checked against `[a-z][a-z0-9_]*`; a name is grammar and is never bound.
    let (namespace, database) = pool.scope();
    // The replication is stated, never left out: a metadata cluster refuses a namespace whose replication nobody said.
    let replication = pool.replication().clause();
    let tenancy = format!(
        "DEFINE NAMESPACE IF NOT EXISTS {namespace} {replication}; USE NAMESPACE {namespace};\n\
         DEFINE DATABASE IF NOT EXISTS {database};"
    );
    pool.run_unscoped(&tenancy).await?;
    pool.run(TABLES, Vec::new()).await?;
    Ok(())
}
