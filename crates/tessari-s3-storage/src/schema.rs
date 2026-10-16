//! The metadata schema, applied at start-up. Every definition says `IF NOT EXISTS`, so a node that restarts
//! re-runs it without effect.

use tessari_s3_constants::{SCHEMA_RETRY_ATTEMPTS, SCHEMA_RETRY_BACKOFF_MS};
use tessari_s3_infrastructure::tessaridb::{MetaError, MetaPool, RefusalClass};

use crate::Result;

/// The table definitions, split between two databases of the configured namespace: the GLOBAL one holds what every
/// region shares — buckets, spaces, users, keys, grants, sign-ins and the console's record — and each region's own
/// database (`<database>_<region>`) holds the object metadata of the region's buckets and the region's cluster, so a
/// region's records can be led and kept in the region. No transaction writes both. The `reclaims` queue holds the data ids
/// whose files may be reclaimed: an upload's own id from before its file exists until the commit that references it,
/// and — through the `supersede` event, inside the very transaction that overwrites or deletes an object — the id it
/// replaced. Each record is held back until `due`, the queuing instant plus the database parameter `$reclaim_grace`
/// (defined here from configuration at every start; the events read it, and their bodies are replaced at every start
/// too, so a store from an earlier build gets this build's); its `reclaiming` mark is the fence a commit asserts against, set by a reclaimer before it removes a byte (ADR-0009).
/// Multipart uploads keep their own records (`uploads`, `parts`), never seen by an object read or a listing, and a
/// replaced or removed part queues its data the same way (`part_superseded`). A completed upload's parts become an
/// object's (`upload`, `parts`); when the object is overwritten or deleted, `parts_released` removes them, and each
/// removal queues its file. Each open upload also has a `pending` record whose id orders the uploads as
/// ListMultipartUploads lists them; whatever deletes the upload drops it in the same transaction (`upload_closed`).
/// The cluster's topology lives here too: `s3_nodes` (one record per node and its zone, refreshed as it registers),
/// `layouts` (one record per layout version — its erasure sets, each node with its zone — created once and never
/// rewritten), `layout_current:'cluster'` (the version new objects are written under) and `layout_staged:'cluster'` (a
/// change an operator staged, applied as the next version by compare-and-set). On a cluster member every data id is
/// erasure-coded and `shard_sets` records the layout its shards were placed under, from the transaction that queues
/// the id until reclamation; the `heal_queue` hands out the data ids acknowledged with fewer than every shard durable
/// (or found damaged by a read), one healer at a time, and the `heal_claims` space holds the expiring claim of the
/// one node healing or reclaiming an id. What operators do through the console is appended to the `console_actions` topic, kept a year; a topic refuses changing or removing a message.
/// A bucket whose `acknowledge` is `local` has its object writes and removals acknowledged once its region's majority
/// holds them (`ACKNOWLEDGE LOCAL MAJORITY`) rather than the namespace's majority; `acknowledge_probes:'region'` records
/// the latest write that proved this region's database takes that acknowledgement.
/// `bucket_usage` holds one record, the latest per-bucket measurement, and `usage_claims` the claim of the member
/// taking it.
/// `object_events` is the notification outbox, a topic: a write or removal stamps the object record with whether its
/// bucket notifies (`notify`) and what wrote it (`op`), and the `object_created` / `object_removed` events append one
/// message per change in the change's own commit, named after the change's data (never a generated identity, which
/// would make concurrent writers contend). A version removed by its id is stamped with its message's name and event
/// in the removal's fence, and `version_removed` appends it. Its position is the event's sequencer.
/// `replication_tasks` is the replication outbox, a topic: a version written into a bucket whose rule for its key
/// replicates it is stamped `replication_status = 'PENDING'` with the destination (`replicate_to`), and the
/// `replication_queued` event appends its task in the version's own commit, named after the bucket incarnation, the
/// key's sequence and the key, so a version is queued exactly once.
/// `lifecycle_runs` holds one record per bucket with lifecycle rules, written in the commit that writes them and due
/// again an interval after each pass; a claim hands a bucket to one node at a time.
/// A bucket names the space that owns it (`space_name`) and, when a user made it, its `creator`; buckets from before
/// spaces existed are moved into the default space by [`apply`] (see [`fill_spaces`]).
/// `s3_regions:<region>` holds the URL a region's S3 surface is reached at, as its nodes advertise it; a replicator
/// sends a version to its destination bucket's region there.
/// `region_failovers:<region>` records a region an operator failed over to another (`promoted`) under its epoch; a
/// failed-over region refuses client writes until a restore, which takes the next epoch again.
/// `spaces`, `users` (one space each), `access_keys` (the secret sealed, never plain) and `grants` (a user's access to
/// one bucket, id `<user>/<bucket>`) hold the access model; the indexes serve a space's users and a user's keys and
/// grants. A one-key upload credential is an access key with `scope_bucket`, `scope_key` and `expires`; the table
/// lets a record carry an expiry, so the store drops such a key at that instant, while a full key carries none.
/// `key_issues:<token>` remembers which key a console form's token issued, so a repeated submission answers that key;
/// the store drops it once its window has passed.
/// `oidc_logins:<state>` holds a console sign-in sent to the identity provider until its callback takes it (the store
/// drops it at its expiry), and `identity_links` which user each provider identity signs in as.
/// The definitions commit as ONE transaction: a node never sees the tables without the event, and nodes starting
/// together contend once per attempt rather than once per definition.
const GLOBAL_TABLES: &str = "\
BEGIN;
DEFINE TABLE IF NOT EXISTS buckets (\
 name string REQUIRED, created datetime REQUIRED, region string REQUIRED, incarnation uuid REQUIRED);
DEFINE FIELD IF NOT EXISTS space_name ON buckets TYPE string DEFAULT 'default';
DEFINE FIELD IF NOT EXISTS creator ON buckets TYPE string;
DEFINE FIELD IF NOT EXISTS max_bytes ON buckets TYPE int;
DEFINE FIELD IF NOT EXISTS max_objects ON buckets TYPE int;
DEFINE FIELD IF NOT EXISTS versioning ON buckets TYPE 'Enabled' | 'Suspended';
DEFINE FIELD IF NOT EXISTS lifecycle ON buckets TYPE array;
DEFINE FIELD IF NOT EXISTS replication ON buckets TYPE object;
DEFINE FIELD IF NOT EXISTS notify ON buckets TYPE object;
DEFINE FIELD IF NOT EXISTS media ON buckets TYPE array;
DEFINE FIELD IF NOT EXISTS acknowledge ON buckets TYPE 'local';
DEFINE FIELD IF NOT EXISTS object_lock ON buckets TYPE object;
DEFINE FIELD IF NOT EXISTS tags ON buckets TYPE object;
DEFINE FIELD IF NOT EXISTS encryption ON buckets TYPE object;
DEFINE FIELD IF NOT EXISTS policy ON buckets TYPE string;
DEFINE FIELD IF NOT EXISTS public_access ON buckets TYPE string;
DEFINE EVENT OR REPLACE object_lock_kept ON buckets FOR UPDATE \
WHEN $before.object_lock != NONE AND ($after.object_lock = NONE OR $after.versioning != 'Enabled') \
THEN THROW 'ObjectLockKept: a bucket with Object Lock keeps it, and keeps versioning enabled';
DEFINE INDEX IF NOT EXISTS by_space ON buckets FIELDS space_name;
DEFINE TOPIC IF NOT EXISTS console_actions RETAIN 365d;
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
DEFINE TABLE IF NOT EXISTS oidc_logins (nonce string REQUIRED, verifier string REQUIRED);
ALTER TABLE oidc_logins SET EXPIRE;
DEFINE TABLE IF NOT EXISTS identity_links (issuer string REQUIRED, subject string REQUIRED, user_name string REQUIRED);
DEFINE INDEX IF NOT EXISTS by_link_user ON identity_links FIELDS user_name;
DEFINE TABLE IF NOT EXISTS console_logouts (after int REQUIRED);
DEFINE TABLE IF NOT EXISTS oidc_logout_ids (seen int REQUIRED);
ALTER TABLE oidc_logout_ids SET EXPIRE;
DEFINE TABLE IF NOT EXISTS s3_regions (endpoint string REQUIRED, seen datetime REQUIRED);
DEFINE TABLE IF NOT EXISTS region_failovers (region string REQUIRED, epoch int REQUIRED, promoted string, \
 reason string REQUIRED, actor string REQUIRED, at datetime REQUIRED);
COMMIT;
";

/// The region's tables, run inside its own database; `__GLOBAL__` stands for the global database, whose `buckets`
/// an event reads by its qualified name.
const REGIONAL_TABLES: &str = "\
BEGIN;
DEFINE TABLE IF NOT EXISTS acknowledge_probes (checked datetime REQUIRED);
DEFINE TABLE IF NOT EXISTS objects (\
 bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED, size int REQUIRED, etag string REQUIRED,\
 modified datetime REQUIRED, headers object REQUIRED, metadata object REQUIRED, checksums object REQUIRED,\
 data uuid, upload uuid, parts int);
DEFINE PARAM OR REPLACE $reclaim_grace VALUE $grace;
DEFINE QUEUE IF NOT EXISTS reclaims TIMEOUT 30s SCHEMAFULL NOT BEFORE due;
DEFINE FIELD IF NOT EXISTS data ON reclaims TYPE uuid REQUIRED;
DEFINE FIELD IF NOT EXISTS due ON reclaims TYPE datetime REQUIRED;
DEFINE FIELD IF NOT EXISTS reclaiming ON reclaims TYPE datetime;
DEFINE INDEX IF NOT EXISTS by_data ON objects FIELDS data;
DEFINE INDEX IF NOT EXISTS by_bucket ON objects FIELDS bucket_name;
DEFINE EVENT OR REPLACE supersede ON objects FOR UPDATE, DELETE \
WHEN $before.data != NONE AND $before.data != $after.data \
THEN { LET $old = $before.data; \
UPSERT reclaims:$old SET data = $old, due = time::now() + duration::from_secs($reclaim_grace); };
DEFINE TABLE IF NOT EXISTS uploads (\
 bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED, initiated datetime REQUIRED,\
 headers object REQUIRED, metadata object REQUIRED, checksum_algorithm string, checksum_type string,\
 last_part datetime, position string);
DEFINE TABLE IF NOT EXISTS parts (\
 upload uuid REQUIRED, number int REQUIRED, data uuid REQUIRED, size int REQUIRED, etag string REQUIRED,\
 modified datetime REQUIRED, checksums object REQUIRED);
DEFINE INDEX IF NOT EXISTS by_upload ON parts FIELDS upload;
DEFINE INDEX IF NOT EXISTS by_part_data ON parts FIELDS data;
DEFINE EVENT OR REPLACE part_superseded ON parts FOR UPDATE, DELETE \
WHEN $before.data != NONE AND $before.data != $after.data \
THEN { LET $old = $before.data; \
UPSERT reclaims:$old SET data = $old, due = time::now() + duration::from_secs($reclaim_grace); };
DEFINE FIELD IF NOT EXISTS version_id ON objects TYPE string;
DEFINE TABLE IF NOT EXISTS versions (\
 bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED, version_id string REQUIRED,\
 seq int REQUIRED, marker bool REQUIRED, modified datetime REQUIRED, size int, etag string, headers object,\
 metadata object, checksums object, data uuid, upload uuid, parts int);
DEFINE TABLE IF NOT EXISTS version_nulls (seq int REQUIRED);
DEFINE TABLE IF NOT EXISTS s3_conversions (done datetime REQUIRED);
DEFINE INDEX IF NOT EXISTS by_version_data ON versions FIELDS data;
DEFINE EVENT OR REPLACE version_released ON versions FOR UPDATE, DELETE \
WHEN $before.data != NONE AND $before.data != $after.data \
THEN { LET $old = $before.data; \
UPSERT reclaims:$old SET data = $old, due = time::now() + duration::from_secs($reclaim_grace); };
DEFINE EVENT IF NOT EXISTS version_parts_released ON versions FOR UPDATE, DELETE \
WHEN $before.upload != NONE AND $before.upload != $after.upload \
THEN { LET $old = $before.upload; DELETE FROM parts WHERE upload = $old LIMIT ALL; };
DEFINE FIELD IF NOT EXISTS lock_mode ON versions TYPE 'COMPLIANCE' | 'GOVERNANCE';
DEFINE FIELD IF NOT EXISTS retain_until ON versions TYPE datetime;
DEFINE FIELD IF NOT EXISTS legal_hold ON versions TYPE bool;
DEFINE FIELD IF NOT EXISTS lock_bypass ON versions TYPE bool;
DEFINE FIELD IF NOT EXISTS tags ON versions TYPE object;
DEFINE FIELD IF NOT EXISTS sse_alg ON objects TYPE string;
DEFINE FIELD IF NOT EXISTS sse_kek ON objects TYPE string;
DEFINE FIELD IF NOT EXISTS sse_nonce ON objects TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_key ON objects TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_c_salt ON objects TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_c_verifier ON objects TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_alg ON versions TYPE string;
DEFINE FIELD IF NOT EXISTS sse_kek ON versions TYPE string;
DEFINE FIELD IF NOT EXISTS sse_nonce ON versions TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_key ON versions TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_c_salt ON versions TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_c_verifier ON versions TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse ON uploads TYPE string;
DEFINE FIELD IF NOT EXISTS sse_kms ON uploads TYPE string;
DEFINE FIELD IF NOT EXISTS sse_c_salt ON uploads TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_c_verifier ON uploads TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_alg ON parts TYPE string;
DEFINE FIELD IF NOT EXISTS sse_kek ON parts TYPE string;
DEFINE FIELD IF NOT EXISTS sse_nonce ON parts TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_key ON parts TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_c_salt ON parts TYPE bytes;
DEFINE FIELD IF NOT EXISTS sse_c_verifier ON parts TYPE bytes;
DEFINE INDEX IF NOT EXISTS by_sse_kek ON versions FIELDS sse_kek;
DEFINE INDEX IF NOT EXISTS by_sse_kek ON objects FIELDS sse_kek;
DEFINE INDEX IF NOT EXISTS by_sse_kek ON parts FIELDS sse_kek;
DEFINE FIELD IF NOT EXISTS tags ON objects TYPE object;
DEFINE FIELD IF NOT EXISTS tags ON uploads TYPE object;
DEFINE EVENT OR REPLACE locked_release ON versions FOR DELETE \
WHEN ($before.legal_hold ?? false) OR ($before.retain_until != NONE AND $before.retain_until > time::now() \
 AND ($before.lock_mode = 'COMPLIANCE' OR ($before.lock_bypass ?? false) = false)) \
THEN THROW 'ObjectLocked: this version is under retention or a legal hold';
DEFINE EVENT OR REPLACE locked_change ON versions FOR UPDATE \
WHEN ($before.retain_until != NONE AND $before.retain_until > time::now() \
  AND ($before.lock_mode = 'COMPLIANCE' OR ($before.lock_bypass ?? false) = false) \
  AND ($after.retain_until = NONE OR $after.retain_until < $before.retain_until \
   OR ($before.lock_mode = 'COMPLIANCE' AND $after.lock_mode != 'COMPLIANCE') \
   OR $after.data != $before.data OR $after.upload != $before.upload)) \
 OR (($before.legal_hold ?? false) AND ($after.data != $before.data OR $after.upload != $before.upload)) \
THEN THROW 'ObjectLocked: this version is under retention or a legal hold';
DEFINE TABLE IF NOT EXISTS pending (\
 upload uuid REQUIRED, bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED,\
 initiated datetime REQUIRED, position string REQUIRED);
DEFINE INDEX IF NOT EXISTS by_pending_initiated ON pending FIELDS initiated;
DEFINE INDEX IF NOT EXISTS by_pending_initiated ON pending FIELDS initiated;
DEFINE EVENT IF NOT EXISTS upload_closed ON uploads FOR DELETE WHEN $before.position != NONE \
THEN { LET $listed = $before.position; DELETE pending:$listed; };
DEFINE TABLE IF NOT EXISTS s3_nodes (node string REQUIRED, endpoint string REQUIRED, zone string, seen datetime REQUIRED);
DEFINE TABLE IF NOT EXISTS s3_members (node string REQUIRED, state 'approved' | 'removed' REQUIRED, \
 fingerprint string, token_digest string, token_expires datetime, reason string REQUIRED, actor string REQUIRED,\
 at datetime REQUIRED);
DEFINE TABLE IF NOT EXISTS layouts (\
 version int REQUIRED, data int REQUIRED, parity int REQUIRED, sets array REQUIRED, created datetime REQUIRED);
DEFINE TABLE IF NOT EXISTS layout_current (version int REQUIRED);
DEFINE TABLE IF NOT EXISTS layout_staged (id string REQUIRED, base int REQUIRED, sets array REQUIRED, staged datetime REQUIRED);
DEFINE TABLE IF NOT EXISTS shard_sets (layout int REQUIRED, size int REQUIRED);
DEFINE INDEX IF NOT EXISTS by_layout ON shard_sets FIELDS layout;
DEFINE FIELD IF NOT EXISTS sealed ON shard_sets TYPE bool;
DEFINE TABLE IF NOT EXISTS layout_retire (old int REQUIRED, switched datetime REQUIRED);
DEFINE INDEX IF NOT EXISTS by_switched ON layout_retire FIELDS switched;
DEFINE QUEUE IF NOT EXISTS heal_queue TIMEOUT 30s SCHEMAFULL NOT BEFORE due;
DEFINE FIELD IF NOT EXISTS data ON heal_queue TYPE uuid REQUIRED;
DEFINE FIELD IF NOT EXISTS due ON heal_queue TYPE datetime REQUIRED;
DEFINE SPACE IF NOT EXISTS heal_claims;
DEFINE FIELD IF NOT EXISTS notify ON objects TYPE bool;
DEFINE FIELD IF NOT EXISTS op ON objects TYPE string;
DEFINE TOPIC IF NOT EXISTS object_events RETAIN 7d;
DEFINE TOPIC IF NOT EXISTS object_events_dead RETAIN 30d;
DEFINE TOPIC IF NOT EXISTS s3_notifications RETAIN 7d;
DEFINE TOPIC IF NOT EXISTS s3_access_log RETAIN 30d;
DEFINE TOPIC IF NOT EXISTS media_failed RETAIN 30d;
DEFINE QUEUE IF NOT EXISTS media_images TIMEOUT 300s ATTEMPTS 5 SCHEMAFULL NOT BEFORE retry_at DEDUPLICATE 7d;
DEFINE QUEUE IF NOT EXISTS media_videos TIMEOUT 3600s ATTEMPTS 5 SCHEMAFULL NOT BEFORE retry_at DEDUPLICATE 7d;
DEFINE FIELD IF NOT EXISTS bucket_name ON media_images TYPE string REQUIRED;
DEFINE FIELD IF NOT EXISTS key ON media_images TYPE string REQUIRED;
DEFINE FIELD IF NOT EXISTS version_id ON media_images TYPE string;
DEFINE FIELD IF NOT EXISTS source ON media_images TYPE uuid REQUIRED;
DEFINE FIELD IF NOT EXISTS output ON media_images TYPE string REQUIRED;
DEFINE FIELD IF NOT EXISTS retry_at ON media_images TYPE datetime REQUIRED;
DEFINE FIELD IF NOT EXISTS error ON media_images TYPE string;
DEFINE FIELD IF NOT EXISTS bucket_name ON media_videos TYPE string REQUIRED;
DEFINE FIELD IF NOT EXISTS key ON media_videos TYPE string REQUIRED;
DEFINE FIELD IF NOT EXISTS version_id ON media_videos TYPE string;
DEFINE FIELD IF NOT EXISTS source ON media_videos TYPE uuid REQUIRED;
DEFINE FIELD IF NOT EXISTS output ON media_videos TYPE string REQUIRED;
DEFINE FIELD IF NOT EXISTS retry_at ON media_videos TYPE datetime REQUIRED;
DEFINE FIELD IF NOT EXISTS error ON media_videos TYPE string;
DEFINE EVENT OR REPLACE object_created ON objects FOR CREATE, UPDATE \
WHEN $after.notify = true AND $after.op != NONE AND ($after.data ?? $after.upload) != ($before.data ?? $before.upload) \
THEN { LET $named = string::concat('c-', type::string($after.data ?? $after.upload)); \
CREATE object_events:$named = { bucket_name: $after.bucket_name, key: $after.key, \
event: string::concat('s3:ObjectCreated:', $after.op), version_id: $after.version_id, size: $after.size, \
etag: $after.etag, at: $after.modified }; };
DEFINE EVENT OR REPLACE object_removed ON objects FOR DELETE WHEN $before.notify = true \
THEN { LET $removed_from = $before.bucket_name; LET $state = (SELECT versioning FROM ONLY __GLOBAL__.buckets:$removed_from); \
LET $named = string::concat('r-', type::string($before.data ?? $before.upload)); \
CREATE object_events:$named = { bucket_name: $before.bucket_name, key: $before.key, \
event: string::concat(string::concat('s3:', $before.op ?? 'ObjectRemoved'), \
IF $state.versioning != NONE THEN ':DeleteMarkerCreated' ELSE ':Delete' END), at: time::now() }; };
DEFINE FIELD IF NOT EXISTS removal ON versions TYPE string;
DEFINE FIELD IF NOT EXISTS removal_event ON versions TYPE string;
DEFINE EVENT OR REPLACE version_removed ON versions FOR DELETE WHEN $before.removal != NONE \
THEN { LET $named = $before.removal; \
CREATE object_events:$named = { bucket_name: $before.bucket_name, key: $before.key, event: $before.removal_event, \
version_id: $before.version_id, at: time::now() }; };
DEFINE FIELD IF NOT EXISTS replication_status ON objects TYPE string;
DEFINE FIELD IF NOT EXISTS replication_status ON versions TYPE string;
DEFINE FIELD IF NOT EXISTS replicate_to ON versions TYPE string;
DEFINE TOPIC IF NOT EXISTS replication_tasks RETAIN 30d;
DEFINE TOPIC IF NOT EXISTS replication_failed RETAIN 30d;
DEFINE EVENT OR REPLACE replication_queued ON versions FOR CREATE WHEN $after.replicate_to != NONE \
THEN { LET $named = string::concat('p-', string::concat(type::string($after.incarnation), string::concat('/', \
string::concat(type::string($after.seq), string::concat('/', $after.key))))); \
CREATE replication_tasks:$named = { bucket_name: $after.bucket_name, key: $after.key, \
version_id: $after.version_id, seq: $after.seq, marker: $after.marker, destination: $after.replicate_to, \
at: $after.modified }; };
DEFINE QUEUE IF NOT EXISTS lifecycle_runs TIMEOUT 300s SCHEMAFULL NOT BEFORE due;
DEFINE FIELD IF NOT EXISTS name ON lifecycle_runs TYPE string REQUIRED;
DEFINE FIELD IF NOT EXISTS due ON lifecycle_runs TYPE datetime REQUIRED;
DEFINE TABLE IF NOT EXISTS bucket_usage (taken datetime REQUIRED, buckets array REQUIRED);
DEFINE SPACE IF NOT EXISTS usage_claims;
COMMIT;
";

/// Creates the namespace, the database and the tables when they are missing. Several nodes starting together race
/// on the catalog and the store answers the loser `retry` rather than waiting, so a `retry` refusal is retried a
/// bounded number of times; every other refusal ends the attempt.
///
/// # Errors
/// The metadata store's refusal or outage, or the last `retry` refusal when every attempt lost.
pub async fn apply(global: &MetaPool, regional: &MetaPool, reclaim_grace_secs: u64) -> Result<()> {
    let mut attempt: u32 = 1;
    loop {
        match apply_once(global, regional, reclaim_grace_secs).await {
            Err(crate::Error::Meta(error))
                if error.is_class(RefusalClass::Retry) && attempt < SCHEMA_RETRY_ATTEMPTS =>
            {
                tracing::debug!(attempt, error = %error, "schema contended; retrying");
                tokio::time::sleep(crate::backoff::backoff(attempt, SCHEMA_RETRY_BACKOFF_MS)).await;
                attempt = attempt.saturating_add(1);
            }
            outcome => return outcome,
        }
    }
}

/// Queues data `$data` for reclamation, held back by the grace the schema recorded — the one statement every
/// server-side path queues with, so it and the events compute `due` from the same value.
pub(crate) const QUEUE_FOR_RECLAIM: &str = "UPSERT reclaims:$data SET data = $data, due = time::now() + duration::from_secs($reclaim_grace);";

/// The schema's one parameter: the reclamation grace, in seconds.
fn grace(
    reclaim_grace_secs: u64,
) -> Result<Vec<(String, tessari_s3_infrastructure::tessaridb::Value)>> {
    let seconds = i64::try_from(reclaim_grace_secs).map_err(|_| crate::Error::Malformed {
        record: "reclaim grace",
        reason: "a grace beyond i64",
    })?;
    Ok(vec![(
        "grace".to_owned(),
        tessari_s3_infrastructure::tessaridb::Value::from(seconds),
    )])
}

async fn apply_once(global: &MetaPool, regional: &MetaPool, reclaim_grace_secs: u64) -> Result<()> {
    // The names come from configuration checked against `[a-z][a-z0-9_]*`; a name is grammar and is never bound.
    let (namespace, database) = global.scope();
    let (_, region_database) = regional.scope();
    // The replication is stated, never left out: a metadata cluster refuses a namespace whose replication nobody said.
    // `MAJORITY OR WEAKER` lets a bucket ask for its writes to be acknowledged by its region's majority alone.
    let replication = global.replication().clause();
    // The server's own account is an editor of its namespace (an operator declares the namespace and the account):
    // `USE` plus each database's `IF NOT EXISTS` is all it may run here. Only when that is refused — the namespace is
    // new, so an account that may declare one is starting the server — is the namespace declared.
    let databases = format!(
        "DEFINE DATABASE IF NOT EXISTS {database}; DEFINE DATABASE IF NOT EXISTS {region_database};"
    );
    let own = format!("USE NAMESPACE {namespace}; {databases}");
    match global.run_unscoped(&own).await {
        Ok(_) => {}
        Err(MetaError::Refused { .. }) => {
            let tenancy = format!(
                "DEFINE NAMESPACE IF NOT EXISTS {namespace} {replication} ACKNOWLEDGE MAJORITY OR WEAKER;\n\
                 USE NAMESPACE {namespace}; {databases}"
            );
            global.run_unscoped(&tenancy).await?;
        }
        Err(error) => return Err(error.into()),
    }
    global.run(GLOBAL_TABLES, Vec::new()).await?;
    let regional_tables = REGIONAL_TABLES.replace("__GLOBAL__", database);
    regional
        .run(&regional_tables, grace(reclaim_grace_secs)?)
        .await?;
    fill_spaces(global).await
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
