//! The internal shard routes through the router with no network: a shard stored, read by block range and removed;
//! and every request whose certificate does not speak for the node it names, or whose node is not an admitted member,
//! refused with nothing left behind.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::Request;
use futures_util::future::BoxFuture;
use rustls::pki_types::CertificateDer;
use tessari_s3_api::{Admit, InternalState, PeerCertificate, internal_router};
use tessari_s3_constants::{INTERNAL_NODE_HEADER, INTERNAL_SHARDS_PATH};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_storage::Storage;
use tessari_s3_storage::members::{Admission, Fingerprint};
use tessari_s3_types::NodeId;
use tower::ServiceExt;

use crate::NOW;

const ID: &str = "0123456789abcdef0123456789abcdef";

/// The membership the router asks, answering what a test sets: 0 admitted, 1 unknown, 2 removed, 3 awaiting a token,
/// 4 another certificate, anything else the store failing.
#[derive(Default)]
pub(crate) struct Admissions(AtomicU8);

impl Admissions {
    pub(crate) fn answer(&self, code: u8) {
        self.0.store(code, Ordering::SeqCst);
    }
}

impl Admit for Admissions {
    fn admits<'a>(
        &'a self,
        _node: &'a NodeId,
        _fingerprint: &'a Fingerprint,
    ) -> BoxFuture<'a, tessari_s3_storage::Result<Admission>> {
        let answer = match self.0.load(Ordering::SeqCst) {
            0 => Ok(Admission::Admitted),
            1 => Ok(Admission::Unknown),
            2 => Ok(Admission::Removed),
            3 => Ok(Admission::AwaitingToken),
            4 => Ok(Admission::OtherCertificate),
            _ => Err(tessari_s3_storage::Error::Malformed {
                record: "member",
                reason: "the test store failed",
            }),
        };
        Box::pin(async move { answer })
    }
}

/// A certificate for `name` alone; the chain is the TLS stack's question, asked before the router's.
pub(crate) fn certificate(name: &str) -> CertificateDer<'static> {
    let key = rcgen::KeyPair::generate().expect("a key");
    let params = rcgen::CertificateParams::new(vec![name.to_owned()]).expect("parameters");
    params
        .self_signed(&key)
        .expect("a certificate")
        .der()
        .clone()
}

/// An internal state at [`NOW`] over a fresh data directory, which it returns for removal, admitting every member.
fn internal() -> (InternalState, std::path::PathBuf) {
    internal_with(Arc::new(|| NOW), Arc::new(Admissions::default()))
}

/// An internal state by `clock` asking `admissions`, over a fresh data directory, which it returns for removal.
pub(crate) fn internal_with(
    clock: tessari_s3_api::Clock,
    admissions: Arc<Admissions>,
) -> (InternalState, std::path::PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("tessari-s3-data-{}", uuid::Uuid::new_v4().simple()));
    let state = InternalState::new(clock, storage_at(dir.clone()), admissions, 16);
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
        ("TESSARIDB_S3_DATA_DIR", "/nonexistent/tessari-s3-wire"),
    ];
    let config = S3Config::from_lookup(|key| {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_owned())
    })
    .expect("test configuration loads");
    Storage::new(MetaPool::new(config.meta).expect("pool settings"), dir)
}

/// 2.5 blocks of 64 bytes.
fn body() -> Vec<u8> {
    (0..160_u32)
        .map(|i| u8::try_from(i % 251).expect("small"))
        .collect()
}

/// A request for `target` naming node `node` (none: no header) over a connection that presented `certificate`.
fn from(
    method: &str,
    target: &str,
    node: Option<&str>,
    certificate: Option<CertificateDer<'static>>,
    body: Vec<u8>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(target);
    if let Some(node) = node {
        builder = builder.header(INTERNAL_NODE_HEADER, node);
    }
    let mut request = builder.body(Body::from(body)).expect("request");
    request
        .extensions_mut()
        .insert(PeerCertificate(certificate));
    request
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

/// Sends a request from member `n1` presenting its own certificate.
async fn sent(state: &InternalState, method: &str, target: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
    let own = certificate("n1.peer.tessari-s3");
    send(state, from(method, target, Some("n1"), Some(own), body)).await
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

const UNIDENTIFIED: &str =
    r#"{"code":"unidentified","message":"no certificate speaks for the node the request names"}"#;
const NOT_ADMITTED: &str =
    r#"{"code":"not_admitted","message":"the node is not an admitted member of the cluster"}"#;

#[tokio::test]
async fn the_health_route_answers_an_admitted_member_and_refuses_anyone_else() {
    let (state, dir) = internal();
    assert_eq!(
        sent(&state, "GET", "/internal/v1/health", Vec::new()).await,
        (204, Vec::new())
    );
    let health = "/internal/v1/health";
    let refusals = [
        (
            "nothing presented",
            from("GET", health, Some("n1"), None, Vec::new()),
        ),
        (
            "another node's certificate",
            from(
                "GET",
                health,
                Some("n1"),
                Some(certificate("n2.peer.tessari-s3")),
                Vec::new(),
            ),
        ),
        (
            "a client link's certificate",
            from(
                "GET",
                health,
                Some("n1"),
                Some(certificate("n1.client.tessari-s3")),
                Vec::new(),
            ),
        ),
        (
            "no node named",
            from(
                "GET",
                health,
                None,
                Some(certificate("n1.peer.tessari-s3")),
                Vec::new(),
            ),
        ),
        (
            "an invalid node id",
            from(
                "GET",
                health,
                Some("N 1"),
                Some(certificate("n1.peer.tessari-s3")),
                Vec::new(),
            ),
        ),
    ];
    for (why, request) in refusals {
        let (status, bytes) = send(&state, request).await;
        assert_eq!(
            (status, String::from_utf8(bytes).expect("utf-8").as_str()),
            (401, UNIDENTIFIED),
            "{why}"
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn the_drive_route_answers_an_admitted_member_with_its_space_and_refuses_anyone_else() {
    let (state, dir) = internal();
    let (status, bytes) = sent(&state, "GET", "/internal/v1/drive", Vec::new()).await;
    assert_eq!(status, 200);
    let text = String::from_utf8(bytes).expect("utf-8");
    let drive = tessari_s3_storage::data::DriveSpace::from_wire(&text).expect("three byte counts");
    assert!(drive.capacity > 0 && drive.free <= drive.capacity, "{text}");
    let (status, _) = send(
        &state,
        from("GET", "/internal/v1/drive", Some("n1"), None, Vec::new()),
    )
    .await;
    assert_eq!(status, 401);
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_node_that_is_not_an_admitted_member_is_refused_and_writes_nothing() {
    let admissions = Arc::new(Admissions::default());
    let (state, dir) = internal_with(Arc::new(|| NOW), Arc::clone(&admissions));
    let put = shard(0, "?block=64&size=160");
    // Each answer under its own node, so no answer is served from an earlier one's cache.
    for (code, node) in [(1, "n3"), (2, "n4"), (3, "n5"), (4, "n6")] {
        admissions.answer(code);
        let own = certificate(&format!("{node}.peer.tessari-s3"));
        let (status, bytes) = send(&state, from("PUT", &put, Some(node), Some(own), body())).await;
        assert_eq!(
            (status, String::from_utf8(bytes).expect("utf-8").as_str()),
            (403, NOT_ADMITTED),
            "answer {code}"
        );
    }
    admissions.answer(9);
    let own = certificate("n7.peer.tessari-s3");
    let (status, _) = send(&state, from("PUT", &put, Some("n7"), Some(own), body())).await;
    assert_eq!(
        status, 503,
        "membership that cannot be read refuses rather than admits"
    );
    let unsigned = send(&state, from("PUT", &put, Some("n1"), None, body())).await;
    assert_eq!(unsigned.0, 401);
    admissions.answer(0);
    let after = sent(&state, "GET", &shard(0, "?size=160"), Vec::new()).await;
    assert_eq!(after.0, 404, "nothing was written");
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_removal_reaches_this_surface_within_five_seconds() {
    let admissions = Arc::new(Admissions::default());
    let now = Arc::new(AtomicI64::new(NOW));
    let clock = Arc::clone(&now);
    let (state, dir) = internal_with(
        Arc::new(move || clock.load(Ordering::SeqCst)),
        Arc::clone(&admissions),
    );
    // One certificate throughout: the answer is kept per node and certificate.
    let own = certificate("n1.peer.tessari-s3");
    let health = || {
        from(
            "GET",
            "/internal/v1/health",
            Some("n1"),
            Some(own.clone()),
            Vec::new(),
        )
    };
    assert_eq!(send(&state, health()).await.0, 204);
    admissions.answer(2);
    now.store(NOW + 4, Ordering::SeqCst);
    assert_eq!(
        send(&state, health()).await.0,
        204,
        "an answer is kept for up to five seconds"
    );
    now.store(NOW + 5, Ordering::SeqCst);
    assert_eq!(send(&state, health()).await.0, 403);
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
