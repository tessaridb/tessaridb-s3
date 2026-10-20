//! A byte flipped in one node's shard file: reads are answered from the other shards with the object's own bytes, the
//! shard is listed and rewritten as it was, and the node that holds it counts the failed verification.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::requests::{call, fetch};
use crate::s3::{Cluster, count};
use crate::{HEALED_WITHIN, large, meta_from_env};

#[tokio::test]
#[ignore = "needs TESSARIDB_S3_TEST_META, _USER and _PASSWORD and the built binary"]
async fn a_flipped_byte_in_a_shard_is_read_around_healed_and_counted() {
    let cluster = Cluster::start(meta_from_env()).await;
    let front = &cluster.nodes[0];
    assert_eq!(call(front, "PUT", "/rot", b"").await.0, 200);
    let object = large();
    let (status, body) = call(front, "PUT", "/rot/k", &object).await;
    assert_eq!(status, 200, "{body}");
    // The first data shard is read by every whole-object read.
    let (holder, path) = cluster
        .nodes
        .iter()
        .enumerate()
        .find_map(|(index, node)| shard(&node.dir, ".s0").map(|path| (index, path)))
        .expect("a node holds shard 0");
    let original = std::fs::read(&path).expect("shard read");
    let mut damaged = original.clone();
    *damaged.get_mut(132).expect("long enough") ^= 0xff;
    std::fs::write(&path, &damaged).expect("shard damaged");

    let reader = &cluster.nodes[(holder + 1) % cluster.nodes.len()];
    let (status, read) = fetch(reader, "/rot/k").await.expect("the node answers");
    assert_eq!(status, 200);
    assert!(
        read == object,
        "the object's own bytes, read around the damaged shard"
    );

    let started = Instant::now();
    while std::fs::read(&path).ok().as_ref() != Some(&original)
        || count(&cluster.meta, "heal_queue").await > 0
    {
        assert!(
            started.elapsed() < HEALED_WITHIN,
            "not rewritten after {HEALED_WITHIN:?}; see {}",
            cluster.logs.display()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // The HTTP stack needs a TLS provider even for plain HTTP; the node's own, which trusts nothing, is used.
    let plain = tessari_s3_infrastructure::tls::plain_client_config().expect("a TLS stack");
    let text = reqwest::Client::builder()
        .tls_backend_preconfigured(plain)
        .build()
        .expect("an HTTP client")
        .get(format!("http://{}/metrics", cluster.nodes[holder].metrics))
        .send()
        .await
        .expect("metrics answer")
        .text()
        .await
        .expect("metrics text");
    let failed = text
        .lines()
        .find_map(|line| line.strip_prefix("s3_data_verification_failures_total "))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .expect("the counter is exported");
    assert!(failed >= 1, "counted on the node holding the shard: {text}");
    // The heal that rewrote the shard ran as a daemon, and the running node counts its runs.
    assert!(
        text.lines().any(|line| line
            .starts_with("s3_daemon_runs_total{daemon=\"healer\",outcome=\"succeeded\"} ")),
        "the healer's runs are counted: {text}"
    );
}

/// The file under `dir` whose name ends with `suffix`.
fn shard(dir: &Path, suffix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        if path.is_dir() {
            shard(&path, suffix)
        } else {
            path.to_string_lossy().ends_with(suffix).then_some(path)
        }
    })
}
