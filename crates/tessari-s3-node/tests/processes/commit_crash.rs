//! A node stopped inside a commit, as a power cut would stop it. Stopped with the object's bytes durable and its record
//! not written, the restarted cluster shows no object at that key and reclaims every shard the write left; stopped
//! with the record written and the client not yet answered, it shows the object whole.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::requests::{call, fetch, put};
use crate::s3::{Cluster, count};
use crate::{large, meta_from_env};

/// How long the stopped node's orphaned shards may take to be reclaimed, with a one-second grace and interval.
const RECLAIMED_WITHIN: Duration = Duration::from_secs(60);

/// The node's variables for these runs: its reclaimer looks every second and reclaims what is a second old.
const QUICK_RECLAIM: [(&str, &str); 2] = [
    ("TESSARIDB_S3_RECLAIM_GRACE_SECS", "1"),
    ("TESSARIDB_S3_RECLAIM_INTERVAL_SECS", "1"),
];

/// Every shard file under `dir` and its subdirectories.
fn shards(dir: &Path, found: &mut BTreeSet<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            shards(&path, found);
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.starts_with('s'))
        {
            found.insert(path);
        }
    }
}

fn every_shard(cluster: &Cluster) -> BTreeSet<std::path::PathBuf> {
    let mut found = BTreeSet::new();
    for node in &cluster.nodes {
        shards(&node.dir, &mut found);
    }
    found
}

/// Restarts node 0 armed to stop at `point`, sends it a write of `object` to `path`, and waits for it to stop.
async fn stop_inside_a_commit(cluster: &mut Cluster, point: &str, path: &str, object: &[u8]) {
    crate::disk::kill(cluster.nodes[0].child.id().expect("running")).await;
    let _reaped = cluster.nodes[0].child.wait().await;
    let mut armed = vec![("TESSARIDB_S3_TEST_EXIT_AT", point)];
    armed.extend(QUICK_RECLAIM);
    cluster.restart_with(0, &armed).await;
    // The node stops before it answers, so the request ends without a response.
    let answered = put(&cluster.nodes[0], path, object).await;
    assert!(
        answered.is_err(),
        "answered although stopped at {point}: {answered:?}"
    );
    let stopped = tokio::time::timeout(Duration::from_secs(30), cluster.nodes[0].child.wait())
        .await
        .expect("the node stopped")
        .expect("its status");
    assert!(!stopped.success(), "stopped at {point}: {stopped}");
    cluster.restart_with(0, &QUICK_RECLAIM).await;
}

#[tokio::test]
#[ignore = "needs TESSARIDB_S3_TEST_META, _USER and _PASSWORD and the built binary"]
async fn a_node_stopped_inside_a_commit_leaves_no_torn_object_and_no_orphan() {
    let mut cluster = Cluster::start(meta_from_env()).await;
    assert_eq!(call(&cluster.nodes[1], "PUT", "/crash", b"").await.0, 200);
    let object = large();

    let before = every_shard(&cluster);
    stop_inside_a_commit(&mut cluster, "before-commit", "/crash/orphan", &object).await;
    let reader = &cluster.nodes[1];
    let (status, _) = fetch(reader, "/crash/orphan")
        .await
        .expect("the node answers");
    assert_eq!(status, 404, "no object without its record");
    let left: Vec<_> = every_shard(&cluster).difference(&before).cloned().collect();
    assert!(!left.is_empty(), "the stopped write had put shards down");
    let started = Instant::now();
    while left.iter().any(|path| path.exists()) || count(&cluster.meta, "reclaims").await > 0 {
        assert!(
            started.elapsed() < RECLAIMED_WITHIN,
            "orphaned shards not reclaimed after {RECLAIMED_WITHIN:?}; see {}",
            cluster.logs.display()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    stop_inside_a_commit(&mut cluster, "after-commit", "/crash/kept", &object).await;
    let (status, read) = fetch(&cluster.nodes[1], "/crash/kept")
        .await
        .expect("the node answers");
    assert_eq!(status, 200, "the committed object is visible");
    assert!(read == object, "the committed object reads back whole");
    let (status, read) = fetch(&cluster.nodes[0], "/crash/kept")
        .await
        .expect("the restarted node answers");
    assert_eq!(status, 200);
    assert!(read == object, "and through the node that stopped");
}
