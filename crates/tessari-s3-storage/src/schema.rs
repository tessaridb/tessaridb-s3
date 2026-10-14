//! The metadata schema, applied at start-up. Every definition says `IF NOT EXISTS`, so a node that restarts
//! re-runs it without effect.

use std::time::Duration;

use tessari_s3_constants::{SCHEMA_RETRY_ATTEMPTS, SCHEMA_RETRY_BACKOFF_MS};
use tessari_s3_infrastructure::tessaridb::{MetaError, MetaPool, RefusalClass};

use crate::Result;

/// The table definitions, run inside the configured namespace and database. The `reclaims` queue holds the data ids
/// whose files may be reclaimed: an upload's own id from before its file exists until the commit that references it,
/// and — through the `supersede` event, inside the very transaction that overwrites or deletes an object — the id it
/// replaced. Each record is held back until `due`, the queuing instant plus the grace in `s3_settings:'reclaim'`
/// (written here from configuration, in the same transaction, because an event takes no parameter); its
/// `reclaiming` mark is the fence a commit asserts against, set by a reclaimer before it removes a byte (ADR-0009).
/// Multipart uploads keep their own records (`uploads`, `parts`), never seen by an object read or a listing, and a
/// replaced or removed part queues its data the same way (`part_superseded`). A completed upload's parts become an
/// object's (`upload`, `parts`); when the object is overwritten or deleted, `parts_released` removes them, and each
/// removal queues its file. Each open upload also has a `pending` record whose id orders the uploads as
/// ListMultipartUploads lists them; whatever deletes the upload drops it in the same transaction (`upload_closed`).
/// The cluster's topology lives here too: `s3_nodes` (one record per node, refreshed as it registers) and `layouts`
/// (one record per layout version, created once and never rewritten). On a cluster member every data id is
/// erasure-coded and `shard_sets` records the layout its shards were placed under, from the transaction that queues
/// the id until reclamation; the `heal_queue` hands out the data ids acknowledged with fewer than every shard durable
/// (or found damaged by a read), one healer at a time, and the `heal_claims` space holds the expiring claim of the
/// one node healing or reclaiming an id. What operators do through the console is appended to the `console_actions` topic, kept a year; a topic refuses changing or removing a message.
/// `bucket_usage` holds one record, the latest per-bucket measurement, and `usage_claims` the claim of the member
/// taking it.
/// A bucket names the space that owns it (`space_name`) and, when a user made it, its `creator`; buckets from before
/// spaces existed are moved into the default space by [`apply`] (see [`fill_spaces`]).
/// `spaces`, `users` (one space each), `access_keys` (the secret sealed, never plain) and `grants` (a user's access to
/// one bucket, id `<user>/<bucket>`) hold the access model; the indexes serve a space's users and a user's keys and
/// grants. A one-key upload credential is an access key with `scope_bucket`, `scope_key` and `expires`; the table
/// lets a record carry an expiry, so the store drops such a key at that instant, while a full key carries none.
/// `key_issues:<token>` remembers which key a console form's token issued, so a repeated submission answers that key;
/// the store drops it once its window has passed.
/// The definitions commit as ONE transaction: a node never sees the tables without the event, and nodes starting
/// together contend once per attempt rather than once per definition.
const TABLES: &str = "\
BEGIN;
DEFINE TABLE IF NOT EXISTS buckets (\
 name string REQUIRED, created datetime REQUIRED, region string REQUIRED, incarnation uuid REQUIRED);
DEFINE FIELD IF NOT EXISTS space_name ON buckets TYPE string DEFAULT 'default';
DEFINE FIELD IF NOT EXISTS creator ON buckets TYPE string;
DEFINE FIELD IF NOT EXISTS max_bytes ON buckets TYPE int;
DEFINE FIELD IF NOT EXISTS max_objects ON buckets TYPE int;
DEFINE FIELD IF NOT EXISTS versioning ON buckets TYPE 'Enabled' | 'Suspended';
DEFINE FIELD IF NOT EXISTS lifecycle ON buckets TYPE array;
DEFINE INDEX IF NOT EXISTS by_space ON buckets FIELDS space_name;
DEFINE TABLE IF NOT EXISTS objects (\
 bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED, size int REQUIRED, etag string REQUIRED,\
 modified datetime REQUIRED, headers object REQUIRED, metadata object REQUIRED, checksums object REQUIRED,\
 data uuid, upload uuid, parts int);
DEFINE TABLE IF NOT EXISTS s3_settings (grace int REQUIRED);
UPSERT s3_settings:'reclaim' SET grace = $grace;
DEFINE QUEUE IF NOT EXISTS reclaims TIMEOUT 30s SCHEMAFULL NOT BEFORE due;
DEFINE FIELD IF NOT EXISTS data ON reclaims TYPE uuid REQUIRED;
DEFINE FIELD IF NOT EXISTS due ON reclaims TYPE datetime REQUIRED;
DEFINE FIELD IF NOT EXISTS reclaiming ON reclaims TYPE datetime;
DEFINE INDEX IF NOT EXISTS by_data ON objects FIELDS data;
DEFINE INDEX IF NOT EXISTS by_bucket ON objects FIELDS bucket_name;
DEFINE EVENT IF NOT EXISTS supersede ON objects FOR UPDATE, DELETE \
WHEN $before.data != NONE AND $before.data != $after.data \
THEN { LET $old = $before.data; LET $settings = (SELECT grace FROM ONLY s3_settings:'reclaim'); \
LET $grace = $settings.grace; \
UPSERT reclaims:$old SET data = $old, due = time::from_unix(time::unix(time::now()) + $grace); };
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
THEN { LET $old = $before.data; LET $settings = (SELECT grace FROM ONLY s3_settings:'reclaim'); \
LET $grace = $settings.grace; \
UPSERT reclaims:$old SET data = $old, due = time::from_unix(time::unix(time::now()) + $grace); };
DEFINE FIELD IF NOT EXISTS version_id ON objects TYPE string;
DEFINE TABLE IF NOT EXISTS versions (\
 bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED, version_id string REQUIRED,\
 seq int REQUIRED, marker bool REQUIRED, modified datetime REQUIRED, size int, etag string, headers object,\
 metadata object, checksums object, data uuid, upload uuid, parts int);
DEFINE TABLE IF NOT EXISTS version_nulls (seq int REQUIRED);
DEFINE TABLE IF NOT EXISTS s3_conversions (done datetime REQUIRED);
DEFINE INDEX IF NOT EXISTS by_version_data ON versions FIELDS data;
DEFINE EVENT IF NOT EXISTS version_released ON versions FOR UPDATE, DELETE \
WHEN $before.data != NONE AND $before.data != $after.data \
THEN { LET $old = $before.data; LET $settings = (SELECT grace FROM ONLY s3_settings:'reclaim'); \
LET $grace = $settings.grace; \
UPSERT reclaims:$old SET data = $old, due = time::from_unix(time::unix(time::now()) + $grace); };
DEFINE EVENT IF NOT EXISTS version_parts_released ON versions FOR UPDATE, DELETE \
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
DEFINE QUEUE IF NOT EXISTS heal_queue TIMEOUT 30s SCHEMAFULL NOT BEFORE due;
DEFINE FIELD IF NOT EXISTS data ON heal_queue TYPE uuid REQUIRED;
DEFINE FIELD IF NOT EXISTS due ON heal_queue TYPE datetime REQUIRED;
DEFINE SPACE IF NOT EXISTS heal_claims;
DEFINE TOPIC IF NOT EXISTS console_actions RETAIN 365d;
DEFINE TABLE IF NOT EXISTS bucket_usage (taken datetime REQUIRED, buckets array REQUIRED);
DEFINE SPACE IF NOT EXISTS usage_claims;
DEFINE TABLE IF NOT EXISTS spaces (name string REQUIRED, created datetime REQUIRED);
DEFINE TABLE IF NOT EXISTS users (\
 name string REQUIRED, space_name string REQUIRED, space_role string REQUIRED, create_buckets bool REQUIRED,\
 operator bool REQUIRED, cluster_viewer bool REQUIRED, disabled bool REQUIRED, created datetime REQUIRED);
DEFINE INDEX IF NOT EXISTS by_user_space ON users FIELDS space_name;
DEFINE TABLE IF NOT EXISTS access_keys (\
 user_name string REQUIRED, secret bytes REQUIRED, nonce bytes REQUIRED, kek_id string REQUIRED,\
 algorithm string REQUIRED, disabled bool REQUIRED, created datetime REQUIRED);
DEFINE INDEX IF NOT EXISTS by_key_user ON access_keys FIELDS user_name;
DEFINE FIELD IF NOT EXISTS scope_bucket ON access_keys TYPE string;
DEFINE FIELD IF NOT EXISTS scope_key ON access_keys TYPE string;
DEFINE FIELD IF NOT EXISTS expires ON access_keys TYPE datetime;
ALTER TABLE access_keys SET EXPIRE;
DEFINE TABLE IF NOT EXISTS key_issues (user_name string REQUIRED, key_id string REQUIRED);
ALTER TABLE key_issues SET EXPIRE;
DEFINE TABLE IF NOT EXISTS grants (\
 user_name string REQUIRED, bucket_name string REQUIRED, can_read bool REQUIRED, can_write bool REQUIRED);
DEFINE INDEX IF NOT EXISTS by_grant_user ON grants FIELDS user_name;
COMMIT;
";

/// Creates the namespace, the database and the tables when they are missing. Several nodes starting together race
/// on the catalog and the store answers the loser `retry` rather than waiting, so a `retry` refusal is retried a
/// bounded number of times; every other refusal ends the attempt.
///
/// # Errors
/// The metadata store's refusal or outage, or the last `retry` refusal when every attempt lost.
pub async fn apply(pool: &MetaPool, reclaim_grace_secs: u64) -> Result<()> {
    let mut attempt: u32 = 1;
    loop {
        match apply_once(pool, reclaim_grace_secs).await {
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

/// Queues data `$data` for reclamation, held back by the grace the schema recorded — the one statement every
/// server-side path queues with, so it and the events compute `due` from the same value.
pub(crate) const QUEUE_FOR_RECLAIM: &str = "LET $settings = (SELECT grace FROM ONLY s3_settings:'reclaim'); \
LET $grace = $settings.grace; \
UPSERT reclaims:$data SET data = $data, due = time::from_unix(time::unix(time::now()) + $grace);";

/// The schema's one parameter: the reclamation grace, in seconds.
fn grace(
    reclaim_grace_secs: u64,
) -> Result<Vec<(String, tessari_s3_infrastructure::tessaridb::Value)>> {
    let seconds = i64::try_from(reclaim_grace_secs).map_err(|_| crate::Error::Malformed {
        record: "s3_settings",
        reason: "a grace beyond i64",
    })?;
    Ok(vec![(
        "grace".to_owned(),
        tessari_s3_infrastructure::tessaridb::Value::from(seconds),
    )])
}

/// A random value in `0..bound` from the standard library's randomly keyed hasher; no dependency needed for a
/// back-off spread.
fn jitter(bound: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(bound);
    hasher.finish().checked_rem(bound).unwrap_or(0)
}

async fn apply_once(pool: &MetaPool, reclaim_grace_secs: u64) -> Result<()> {
    // The names come from configuration checked against `[a-z][a-z0-9_]*`; a name is grammar and is never bound.
    let (namespace, database) = pool.scope();
    // The replication is stated, never left out: a metadata cluster refuses a namespace whose replication nobody said.
    let replication = pool.replication().clause();
    // The server's own account is an editor of its one database (an operator declares the namespace, the database and
    // the account): `USE` plus the database's `IF NOT EXISTS` is all it may run here. Only when that is refused — the
    // namespace is new, so an account that may declare one is starting the server — is the namespace declared.
    let own = format!("USE NAMESPACE {namespace}; DEFINE DATABASE IF NOT EXISTS {database};");
    match pool.run_unscoped(&own).await {
        Ok(_) => {}
        Err(MetaError::Refused { .. }) => {
            let tenancy = format!(
                "DEFINE NAMESPACE IF NOT EXISTS {namespace} {replication}; USE NAMESPACE {namespace};\n\
             DEFINE DATABASE IF NOT EXISTS {database};"
            );
            pool.run_unscoped(&tenancy).await?;
        }
        Err(error) => return Err(error.into()),
    }
    pool.run(TABLES, grace(reclaim_grace_secs)?).await?;
    fill_spaces(pool).await
}

/// Gives every bucket record without a `space_name` the default space. A field's `DEFAULT` fills only what is written
/// after it is declared, and an index on the field holds no entry for a record lacking it — so the records are found
/// by reading every bucket (a scan; buckets are few) and each is written by its own identity, conditionally, so a
/// bucket created or moved meanwhile is left alone.
async fn fill_spaces(pool: &MetaPool) -> Result<()> {
    use tessari_s3_infrastructure::tessaridb::{Answer, Value};
    let answers = pool
        .run("SELECT name, space_name FROM buckets;", Vec::new())
        .await?;
    let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
        return Err(crate::Error::Malformed {
            record: "bucket",
            reason: "a listing answered no records",
        });
    };
    for (_, value) in records {
        let Value::Object(fields) = value else {
            continue;
        };
        if fields.contains_key("space_name") {
            continue;
        }
        let Some(Value::String(name)) = fields.get("name") else {
            continue;
        };
        let parameters = vec![
            ("name".to_owned(), Value::String(name.clone())),
            (
                "space".to_owned(),
                Value::String(tessari_s3_constants::DEFAULT_SPACE.to_owned()),
            ),
        ];
        pool.run(
            "UPDATE buckets:$name SET space_name = $space WHERE space_name = NONE;",
            parameters,
        )
        .await?;
    }
    Ok(())
}
