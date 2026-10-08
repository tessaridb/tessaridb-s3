//! The internal shard routes through the router with no network: a shard stored, read by block range and removed;
//! and every request that is not the cluster's, or not what it declares, refused with nothing left behind.

use axum::body::{Body, to_bytes};
use axum::http::Request;
use tessari_s3_api::{InternalState, internal_router};
use tessari_s3_constants::{
    INTERNAL_DATE_HEADER, INTERNAL_NODE_HEADER, INTERNAL_SHARDS_PATH, INTERNAL_SIGNATURE_HEADER,
};
use tessari_s3_core::internal::{InternalRequest, sign};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_storage::Storage;
use tessari_s3_types::SecretKey;
use tower::ServiceExt;

use crate::NOW;

pub(crate) const CLUSTER_SECRET: &str = "cluster-secret-0123456789abcdef0123";
const ID: &str = "0123456789abcdef0123456789abcdef";

/// An internal state at [`NOW`] over a fresh data directory, which it returns for removal.
fn internal() -> (InternalState, std::path::PathBuf) {
    internal_with(std::sync::Arc::new(|| NOW))
}

/// An internal state by `clock` over a fresh data directory, which it returns for removal.
pub(crate) fn internal_with(clock: tessari_s3_api::Clock) -> (InternalState, std::path::PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("tessari-s3-data-{}", uuid::Uuid::new_v4().simple()));
    let state = InternalState::new(
        SecretKey::new(CLUSTER_SECRET.to_owned()),
        clock,
        storage_at(dir.clone()),
        16,
    );
    (state, dir)
}

/// Storage over data directory `dir`. The metadata pool points at a port that refuses every connection and is never
/// dialled: shards live on the drive alone.
pub(crate) fn storage_at(dir: std::path::PathBuf) -> Storage {
    let vars = [
        ("TESSARIDB_S3_ROOT_ACCESS_KEY", "AKIAIOSFODNN7EXAMPLE"),
        (
            "TESSARIDB_S3_ROOT_SECRET_KEY",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        ),
        ("TESSARIDB_S3_META_ADDRESS", "127.0.0.1:1"),
        ("TESSARIDB_S3_META_USER", "s3"),
        ("TESSARIDB_S3_META_PASSWORD", "unused-password"),
    ];
    let config = S3Config::from_lookup(|key| {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_owned())
    })
    .expect("test configuration loads");
    Storage::new(
        MetaPool::new(config.meta).expect("pool settings"),
        Some(dir),
    )
}

/// 2.5 blocks of 64 bytes.
fn body() -> Vec<u8> {
    (0..160_u32)
        .map(|i| u8::try_from(i % 251).expect("small"))
        .collect()
}

/// A request for `target` (path and query) signed by node `n1` at `date` with `secret`.
fn signed(method: &str, target: &str, date: i64, secret: &str, body: Vec<u8>) -> Request<Body> {
    let signature = sign(
        secret.as_bytes(),
        &InternalRequest {
            method,
            path: target,
            date,
            node: "n1",
        },
    );
    Request::builder()
        .method(method)
        .uri(target)
        .header(INTERNAL_NODE_HEADER, "n1")
        .header(INTERNAL_DATE_HEADER, date.to_string())
        .header(INTERNAL_SIGNATURE_HEADER, signature)
        .body(Body::from(body))
        .expect("request")
}

fn shard(index: u16, query: &str) -> String {
    format!("{INTERNAL_SHARDS_PATH}/{ID}/{index}{query}")
}

async fn send(state: &InternalState, request: Request<Body>) -> (u16, Vec<u8>) {
    let response = internal_router(state.clone())
        .oneshot(request)
        .await
        .expect("the router is infallible");
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body")
        .to_vec();
    (status, bytes)
}

/// Sends a request signed by the cluster now.
async fn sent(state: &InternalState, method: &str, target: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
    send(state, signed(method, target, NOW, CLUSTER_SECRET, body)).await
}

#[tokio::test]
async fn a_shard_is_stored_read_by_range_and_removed() {
    let (state, dir) = internal();
    let bytes = body();
    let stored = sent(
        &state,
        "PUT",
        &shard(2, "?block=64&size=160"),
        bytes.clone(),
    )
    .await;
    assert_eq!(stored.0, 200);
    let whole = sent(&state, "GET", &shard(2, "?size=160"), Vec::new()).await;
    assert_eq!(whole, (200, bytes.clone()));
    let middle = sent(
        &state,
        "GET",
        &shard(2, "?size=160&from=1&to=2"),
        Vec::new(),
    )
    .await;
    assert_eq!(middle, (200, bytes[64..128].to_vec()));
    let removed = sent(&state, "DELETE", &shard(2, ""), Vec::new()).await;
    assert_eq!(removed.0, 204);
    let gone = sent(&state, "GET", &shard(2, "?size=160"), Vec::new()).await;
    assert_eq!(gone.0, 404);
    assert_eq!(
        String::from_utf8(gone.1).expect("utf-8"),
        r#"{"code":"not_found","message":"no such shard here"}"#
    );
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn the_health_route_answers_the_cluster_and_refuses_anyone_else() {
    let (state, dir) = internal();
    assert_eq!(
        sent(&state, "GET", "/internal/v1/health", Vec::new()).await,
        (204, Vec::new())
    );
    let other = "another-secret-0123456789abcdef0123";
    let (status, bytes) = send(
        &state,
        signed("GET", "/internal/v1/health", NOW, other, Vec::new()),
    )
    .await;
    assert_eq!(status, 401);
    assert_eq!(
        String::from_utf8(bytes).expect("utf-8"),
        r#"{"code":"unauthorized","message":"the request is not signed by the cluster"}"#
    );
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_request_the_cluster_did_not_sign_is_refused_and_writes_nothing() {
    let (state, dir) = internal();
    let put = shard(0, "?block=64&size=160");
    let other = "another-secret-0123456789abcdef0123";
    // Signed for a shorter shard than it asks for: the query is part of what is signed.
    let mut retargeted = signed(
        "PUT",
        &shard(0, "?block=64&size=96"),
        NOW,
        CLUSTER_SECRET,
        body(),
    );
    *retargeted.uri_mut() = put.parse().expect("uri");
    let mut nameless = signed("PUT", &put, NOW, CLUSTER_SECRET, body());
    nameless.headers_mut().remove(INTERNAL_NODE_HEADER);
    let refusals = [
        signed("PUT", &put, NOW, other, body()),
        signed("PUT", &put, NOW - 301, CLUSTER_SECRET, body()),
        signed("PUT", &put, NOW + 301, CLUSTER_SECRET, body()),
        retargeted,
        nameless,
        signed("GET", "/internal/v1/elsewhere", NOW, other, Vec::new()),
    ];
    for (turn, request) in refusals.into_iter().enumerate() {
        let (status, bytes) = send(&state, request).await;
        assert_eq!(status, 401, "refusal {turn}");
        assert_eq!(
            String::from_utf8(bytes).expect("utf-8"),
            r#"{"code":"unauthorized","message":"the request is not signed by the cluster"}"#
        );
    }
    let after = sent(&state, "GET", &shard(0, "?size=160"), Vec::new()).await;
    assert_eq!(after.0, 404, "nothing was written");
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_body_off_its_declared_length_is_refused_and_leaves_nothing() {
    let (state, dir) = internal();
    for size in [161, 159] {
        let target = shard(1, &format!("?block=64&size={size}"));
        assert_eq!(
            sent(&state, "PUT", &target, body()).await.0,
            400,
            "size {size}"
        );
        let read = sent(
            &state,
            "GET",
            &shard(1, &format!("?size={size}")),
            Vec::new(),
        )
        .await;
        assert_eq!(
            read.0, 404,
            "nothing under the final name after size {size}"
        );
    }
    let retry = sent(&state, "PUT", &shard(1, "?block=64&size=160"), body()).await;
    assert_eq!(
        retry.0, 200,
        "no temporary file is left to refuse the retry over"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn malformed_parts_of_a_shard_request_are_refused() {
    let (state, dir) = internal();
    let stored = sent(&state, "PUT", &shard(1, "?block=64&size=160"), body()).await;
    assert_eq!(stored.0, 200);
    let upper = format!(
        "{INTERNAL_SHARDS_PATH}/{}/1?block=64&size=160",
        ID.to_uppercase()
    );
    let refused = [
        ("PUT", upper),
        ("PUT", shard(3, "?block=64&size=160&extra=1")),
        ("PUT", shard(3, "?block=64&size=160&size=160")),
        ("PUT", shard(3, "?block=0&size=160")),
        ("PUT", shard(3, "?block=+64&size=160")),
        ("PUT", shard(16, "?block=64&size=160")),
        ("GET", shard(1, "?size=160&from=2&to=4")),
        ("GET", shard(1, "?size=160&from=2&to=1")),
        ("DELETE", shard(1, "?size=160")),
    ];
    for (method, target) in refused {
        let (status, _) = sent(&state, method, &target, body()).await;
        assert_eq!(status, 400, "{method} {target}");
    }
    std::fs::remove_dir_all(dir).ok();
}
