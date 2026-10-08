//! The bucket operations end to end against a real TessariDB node.
//!
//! Needs `TESSARIDB_S3_TEST_META` (a node's wire `host:port`), `TESSARIDB_S3_TEST_META_USER` and
//! `TESSARIDB_S3_TEST_META_PASSWORD`; each test works in its own fresh namespace. Without them every test is
//! `ignored` — visible in the summary — and `cargo test -p tessari-s3-api --test live -- --ignored` runs them.

#![cfg(test)]

mod access;
mod access_crossing;
mod access_probe;
mod access_window;
mod buckets;
mod cluster;
mod complete;
mod console;
mod console_admin;
mod console_store;
mod console_users;
mod copy;
mod copy_part;
mod delete_many;
mod erasure;
mod erasure_heals;
mod erasure_reads;
mod large;
mod listing;
mod metadata;
mod multipart;
mod objects;
mod reclaim;
#[path = "../wire/signer.rs"]
#[allow(
    dead_code,
    reason = "shared with the wire tests; each binary uses part of it"
)]
mod signer;
mod spaces;
mod uploads;
mod usage;
mod users;
mod users_sealed;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use tessari_s3_api::{ApiState, router};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_storage::Storage;
use tessari_s3_types::Timestamp;
use tower::ServiceExt;

pub(crate) const ACCESS_KEY: &str = "AKLIVETEST0000000001";
pub(crate) const SECRET: &str = "live-test-secret-0123456789abcdef";
/// Schema application, one test at a time.
static SCHEMA_TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) const IGNORED: &str =
    "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD";

/// A server state over a fresh namespace on the test node, schema applied, and a separate pool into the same
/// namespace for a test that must plant a record directly. Large objects go to a fresh data directory that is
/// created on first use.
pub(crate) async fn fresh() -> (ApiState, MetaPool) {
    let (state, planter, _) = fresh_with(Some(scratch_dir())).await;
    (state, planter)
}

/// A directory of this test's own under the system temp dir; the test that inspects it removes it.
pub(crate) fn scratch_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("tessari-s3-live-{}", uuid::Uuid::new_v4().simple()))
}

/// As [`fresh`], with the data directory chosen by the caller (`None`: not configured), which it gets back.
pub(crate) async fn fresh_with(
    data_dir: Option<std::path::PathBuf>,
) -> (ApiState, MetaPool, Option<std::path::PathBuf>) {
    fresh_at(test_node(), None, data_dir).await
}

/// The test node's wire address.
pub(crate) fn test_node() -> String {
    need("TESSARIDB_S3_TEST_META")
}

fn need(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} is required for the live suite"))
}

/// As [`fresh_with`], with the metadata addresses (comma-separated) and replication chosen by the caller.
pub(crate) async fn fresh_at(
    meta_addresses: String,
    replication: Option<&str>,
    data_dir: Option<std::path::PathBuf>,
) -> (ApiState, MetaPool, Option<std::path::PathBuf>) {
    let config = live_config(meta_addresses, replication, data_dir.as_ref());
    let storage = prepared(&config).await;
    let planter = MetaPool::new(config.meta.clone()).expect("pool");
    (
        ApiState::new(&config, ApiState::system_clock(), storage),
        planter,
        data_dir,
    )
}

/// A server state and a console state over ONE storage in a fresh namespace on the test node, with no data
/// directory: what an operator does through the console is what an S3 client then sees.
pub(crate) async fn fresh_console() -> (ApiState, tessari_s3_api::ConsoleState) {
    let config = live_config(test_node(), None, None);
    let storage = prepared(&config).await;
    let console =
        tessari_s3_api::ConsoleState::new(&config, ApiState::system_clock(), storage.clone())
            .expect("a console is configured");
    (
        ApiState::new(&config, ApiState::system_clock(), storage),
        console,
    )
}

/// The configuration of a node over a fresh namespace, its console configured too.
fn live_config(
    meta_addresses: String,
    replication: Option<&str>,
    data_dir: Option<&std::path::PathBuf>,
) -> S3Config {
    let namespace = format!("t_{}", uuid::Uuid::new_v4().simple());
    let mut vars = vec![
        ("TESSARIDB_S3_ROOT_ACCESS_KEY", ACCESS_KEY.to_owned()),
        ("TESSARIDB_S3_ROOT_SECRET_KEY", SECRET.to_owned()),
        ("TESSARIDB_S3_META_ADDRESS", meta_addresses),
        (
            "TESSARIDB_S3_META_USER",
            need("TESSARIDB_S3_TEST_META_USER"),
        ),
        (
            "TESSARIDB_S3_META_PASSWORD",
            need("TESSARIDB_S3_TEST_META_PASSWORD"),
        ),
        ("TESSARIDB_S3_META_NAMESPACE", namespace),
        (
            "TESSARIDB_S3_DATA_DIR",
            data_dir
                .map(|dir| dir.display().to_string())
                .unwrap_or_default(),
        ),
        ("TESSARIDB_S3_CONSOLE_LISTEN", "127.0.0.1:9101".to_owned()),
        // The live suite's own root key for sealing user access-key secrets; it seals nothing outside the test node.
        ("TESSARIDB_S3_IAM_KEY", "7e".repeat(32)),
    ];
    if let Some(replication) = replication {
        vars.push(("TESSARIDB_S3_META_REPLICATION", replication.to_owned()));
    }
    S3Config::from_lookup(|key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()))
        .expect("live configuration")
}

/// The storage `config` describes, schema applied.
async fn prepared(config: &S3Config) -> Storage {
    let storage = Storage::new(
        MetaPool::new(config.meta.clone()).expect("pool"),
        config.data_dir.clone(),
    )
    .with_iam_key(config.iam_key.clone());
    // Containment, not a fix (Q-S3-2): concurrent schema applies in different namespaces conflict on one store-wide
    // catalog record, and thirty tests starting at once exhaust the bounded retry. The tests are not about
    // concurrent start-up, so their setup takes turns; each still gets its own namespace.
    let _turn = SCHEMA_TURN.lock().await;
    storage
        .prepare()
        .await
        .expect("schema applies on the test node");
    storage
}

/// What a client sees.
pub(crate) struct Seen {
    pub(crate) status: u16,
    pub(crate) headers: axum::http::HeaderMap,
    pub(crate) code: Option<String>,
    pub(crate) body: String,
    pub(crate) bytes: Vec<u8>,
}

/// `YYYYMMDDTHHMMSSZ` for now.
pub(crate) fn amz_now_for_tests() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).expect("seconds fit"))
        .expect("after the epoch");
    let iso = Timestamp { seconds, nanos: 0 }.iso8601_millis();
    format!(
        "{}{}{}T{}{}{}Z",
        &iso[0..4],
        &iso[5..7],
        &iso[8..10],
        &iso[11..13],
        &iso[14..16],
        &iso[17..19]
    )
}

/// Signs and sends one request carrying `body`, declared by its SHA-256.
pub(crate) async fn call(
    state: &ApiState,
    method: &str,
    path: &str,
    query: Vec<(&str, Option<&str>)>,
    body: &[u8],
) -> Seen {
    call_with(state, method, path, query, Vec::new(), body).await
}

/// As [`call`], with extra headers that are signed (as every `x-amz-*` header must be).
pub(crate) async fn call_with(
    state: &ApiState,
    method: &str,
    path: &str,
    query: Vec<(&str, Option<&str>)>,
    headers: Vec<(&str, &str)>,
    body: &[u8],
) -> Seen {
    call_as(
        state,
        (ACCESS_KEY, SECRET),
        method,
        path,
        query,
        headers,
        body,
    )
    .await
}

/// As [`call_with`], signed with `credential` (access key, secret) rather than the root credential.
pub(crate) async fn call_as(
    state: &ApiState,
    credential: (&str, &str),
    method: &str,
    path: &str,
    query: Vec<(&str, Option<&str>)>,
    headers: Vec<(&str, &str)>,
    body: &[u8],
) -> Seen {
    let (access_key, secret) = credential;
    let amz_date = amz_now_for_tests();
    let unsigned = signer::Unsigned {
        method,
        host: "localhost:9100",
        path,
        query,
        headers,
        access_key,
        secret,
        region: "us-east-1",
        amz_date: &amz_date,
    };
    let (target, mut headers) = signer::sign_with_body(&unsigned, body);
    headers.push(("content-length".to_owned(), body.len().to_string()));
    let mut builder = Request::builder().method(method).uri(target);
    for (name, value) in &headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let request = builder.body(Body::from(body.to_vec())).expect("request");
    let response = router(state.clone())
        .oneshot(request)
        .await
        .expect("infallible router");
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 64 << 20)
        .await
        .map(|bytes| bytes.to_vec())
        .unwrap_or_default();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let code = body
        .split_once("<Code>")
        .and_then(|(_, rest)| rest.split_once("</Code>"))
        .map(|(c, _)| c.to_owned());
    Seen {
        status,
        headers,
        code,
        body,
        bytes,
    }
}
