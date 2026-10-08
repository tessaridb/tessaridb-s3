//! Six `tessaridb-s3` processes forming one RS(4, 2) cluster over a fresh TessariDB namespace, driven over real HTTP.
//!
//! The first test needs `TESSARIDB_S3_TEST_META` (a node's wire `host:port`), `TESSARIDB_S3_TEST_META_USER` and
//! `TESSARIDB_S3_TEST_META_PASSWORD`; the fault tests in [`faults`] start their own three-node TessariDB cluster in
//! Docker. All are `ignored`, and `cargo test -p tessari-s3-node --test processes -- --ignored` runs them.

#![cfg(test)]

#[path = "../../../tessari-s3-api/tests/wire/signer.rs"]
#[allow(
    dead_code,
    reason = "shared with the api crate's tests; this binary uses part of it"
)]
mod signer;

mod docker;
mod faults;
mod meta;
mod requests;
mod s3;

use std::time::{Duration, Instant};

use requests::call;
use s3::{Cluster, Meta, count, shard_files};

/// How long the restarted cluster may take to heal: passes run every second.
const HEALED_WITHIN: Duration = Duration::from_secs(60);

fn need(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} is required for this test"))
}

/// The one metadata node the environment names.
fn meta_from_env() -> Meta {
    Meta {
        addresses: need("TESSARIDB_S3_TEST_META"),
        user: need("TESSARIDB_S3_TEST_META_USER"),
        password: need("TESSARIDB_S3_TEST_META_PASSWORD"),
        replication: None,
        internal_tls: None,
    }
}

/// 300 KiB: past the inline size, inside one stripe.
fn large() -> Vec<u8> {
    (0..300 * 1024_u32)
        .map(|i| u8::try_from(i % 251).expect("small"))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn six_processes_spread_an_object_and_hold_the_write_quorum() {
    let mut cluster = Cluster::start(meta_from_env()).await;
    let front = 0;
    assert_eq!(
        call(&cluster.nodes[front], "PUT", "/shards", b"").await.0,
        200
    );
    let (status, body) = call(&cluster.nodes[front], "PUT", "/shards/first", &large()).await;
    assert_eq!(status, 200, "{body}");
    let spread: Vec<usize> = cluster
        .nodes
        .iter()
        .map(|node| shard_files(&node.dir))
        .collect();
    assert_eq!(spread, vec![1; 6], "one shard on every node");
    assert_eq!(
        count(&cluster.meta, "heals").await,
        0,
        "every shard durable"
    );

    for down in [2, 4] {
        cluster.nodes[down].child.kill().await.expect("killed");
    }
    let (status, body) = call(&cluster.nodes[front], "PUT", "/shards/second", &large()).await;
    assert_eq!(status, 200, "two down is within the parity: {body}");
    assert_eq!(
        count(&cluster.meta, "heals").await,
        1,
        "the degraded write is listed"
    );

    cluster.nodes[5].child.kill().await.expect("killed");
    let (status, body) = call(&cluster.nodes[front], "PUT", "/shards/third", &large()).await;
    assert_eq!(status, 503, "{body}");
    assert!(body.contains("ServiceUnavailable"), "{body}");
    assert_eq!(
        call(&cluster.nodes[front], "HEAD", "/shards/third", b"")
            .await
            .0,
        404,
        "nothing committed"
    );

    // The killed nodes come back with their drives; the healer writes the shards the second object is missing.
    for down in [2, 4, 5] {
        cluster.restart(down).await;
    }
    let started = Instant::now();
    while count(&cluster.meta, "heals").await > 0 {
        assert!(
            started.elapsed() < HEALED_WITHIN,
            "still listed for healing after {HEALED_WITHIN:?}; see {}",
            cluster.logs.display()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    for down in [2, 4] {
        assert_eq!(
            shard_files(&cluster.nodes[down].dir),
            2,
            "the first object's shard and the healed one"
        );
    }
}
