//! The metadata schema, applied at start-up. Every definition says `IF NOT EXISTS`, so a node that restarts
//! re-runs it without effect.

use std::time::Duration;

use tessari_s3_constants::{SCHEMA_RETRY_ATTEMPTS, SCHEMA_RETRY_BACKOFF_MS};
use tessari_s3_infrastructure::tessaridb::{MetaPool, RefusalClass};

use crate::Result;

/// The table definitions, run inside the configured namespace and database.
const TABLES: &str = "\
DEFINE TABLE IF NOT EXISTS buckets (\
 name string REQUIRED, created datetime REQUIRED, region string REQUIRED, incarnation uuid REQUIRED);
DEFINE TABLE IF NOT EXISTS objects (\
 bucket_name string REQUIRED, key string REQUIRED, incarnation uuid REQUIRED, size int REQUIRED, etag string REQUIRED,\
 modified datetime REQUIRED, headers object REQUIRED, metadata object REQUIRED, checksums object REQUIRED,\
 inline bytes, data uuid);
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
                let backoff = u64::from(attempt).saturating_mul(SCHEMA_RETRY_BACKOFF_MS);
                tokio::time::sleep(Duration::from_millis(backoff)).await;
                attempt = attempt.saturating_add(1);
            }
            outcome => return outcome,
        }
    }
}

async fn apply_once(pool: &MetaPool) -> Result<()> {
    // The names come from configuration checked against `[a-z][a-z0-9_]*`; a name is grammar and is never bound.
    let (namespace, database) = pool.scope();
    let tenancy = format!(
        "DEFINE NAMESPACE IF NOT EXISTS {namespace}; USE NAMESPACE {namespace};\n\
         DEFINE DATABASE IF NOT EXISTS {database};"
    );
    pool.run_unscoped(&tenancy).await?;
    pool.run(TABLES, Vec::new()).await?;
    Ok(())
}
