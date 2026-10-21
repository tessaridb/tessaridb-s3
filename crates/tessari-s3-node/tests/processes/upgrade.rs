//! A rolling upgrade under traffic (G024 C1): six nodes of the previous release (`TESSARIDB_S3_TEST_PREVIOUS`, its
//! `tessaridb-s3` binary) form an RS(4, 2) cluster; while a writer keeps putting objects through every node, each node
//! in turn is stopped with SIGTERM and started again on this build over the same drive. Every write the cluster
//! acknowledged is then read back with its bytes, every drive carries this build's data layout, and a drive marked
//! with another layout stops a node's start by name.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::faults::{MIB, patterned};
use crate::requests::{call, fetch, put_to};
use crate::s3::Cluster;
use crate::{QUIET, meta_from_env, need};

/// What the writer saw: each acknowledged object as (index, length), and how many writes were not acknowledged.
#[derive(Default)]
struct Written {
    acknowledged: Vec<(usize, usize)>,
    refused: usize,
}

/// The length of object `index`: most small, every fourth large enough to span several erasure blocks.
fn length(index: usize) -> usize {
    if index % 4 == 3 { 5 * MIB } else { 64 * 1024 }
}

/// Puts objects through `hosts` in turn until `stop` is set.
async fn write_until(hosts: Vec<String>, stop: Arc<AtomicBool>) -> Written {
    let mut written = Written::default();
    let mut index = 0_usize;
    while !stop.load(Ordering::Acquire) {
        let host = &hosts[index.checked_rem(hosts.len()).unwrap_or_default()];
        let len = length(index);
        match put_to(host, &format!("/rolling/o{index}"), &patterned(len, index)).await {
            Ok(200) => written.acknowledged.push((index, len)),
            _ => written.refused = written.refused.saturating_add(1),
        }
        index = index.saturating_add(1);
    }
    written
}

/// Waits until `cluster` answers a HEAD of the bucket through node `index`.
async fn answering(cluster: &Cluster, index: usize) {
    let started = Instant::now();
    while call(&cluster.nodes[index], "HEAD", "/rolling", b"").await.0 != 200 {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{} never answered after its upgrade; see {}",
            cluster.nodes[index].name,
            cluster.logs.display()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
#[ignore = "needs TESSARIDB_S3_TEST_META, _USER, _PASSWORD and TESSARIDB_S3_TEST_PREVIOUS (the previous release's binary)"]
async fn a_rolling_upgrade_under_traffic_loses_no_acknowledged_write() {
    let _quiet = QUIET.lock().await;
    let previous = PathBuf::from(need("TESSARIDB_S3_TEST_PREVIOUS"));
    let current = PathBuf::from(env!("CARGO_BIN_EXE_tessaridb-s3"));
    let mut cluster = Cluster::start_on(meta_from_env(), &previous).await;
    let (status, body) = call(&cluster.nodes[0], "PUT", "/rolling", b"").await;
    assert_eq!(status, 200, "{body}");
    let hosts: Vec<String> = cluster.nodes.iter().map(|node| node.s3.clone()).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let writer = tokio::spawn(write_until(hosts, Arc::clone(&stop)));
    tokio::time::sleep(Duration::from_secs(2)).await;
    for index in 0..cluster.nodes.len() {
        cluster.stop(index).await;
        // The cluster serves with one node down while it is replaced.
        tokio::time::sleep(Duration::from_secs(1)).await;
        cluster
            .restart_on(index, &current, &[])
            .await
            .unwrap_or_else(|refused| panic!("the upgraded node did not start: {refused}"));
        answering(&cluster, index).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    stop.store(true, Ordering::Release);
    let written = writer.await.expect("the writer finished");
    assert!(
        written.acknowledged.len() >= 20,
        "too few writes to judge: {} acknowledged, {} refused",
        written.acknowledged.len(),
        written.refused
    );
    let mut lost = Vec::new();
    for (position, (index, len)) in written.acknowledged.iter().enumerate() {
        let node = &cluster.nodes[position
            .checked_rem(cluster.nodes.len())
            .unwrap_or_default()];
        match fetch(node, &format!("/rolling/o{index}")).await {
            Ok((200, bytes)) if bytes == patterned(*len, *index) => {}
            other => lost.push(format!(
                "o{index}: {:?}",
                other.map(|(status, bytes)| (status, bytes.len()))
            )),
        }
    }
    println!(
        "upgrade: acknowledged={} | refused={} | read back={}",
        written.acknowledged.len(),
        written.refused,
        written.acknowledged.len().saturating_sub(lost.len())
    );
    assert!(
        lost.is_empty(),
        "acknowledged and lost: {lost:?}; see {}",
        cluster.logs.display()
    );
    for node in &cluster.nodes {
        let marker = std::fs::read_to_string(node.dir.join("s3data/LAYOUT")).expect("marked");
        assert_eq!(marker.trim(), "tessaridb-s3 data layout 1", "{}", node.name);
    }

    // A drive marked with a layout this build does not read stops the start by name and is left as it was.
    cluster.stop(5).await;
    let marker = cluster.nodes[5].dir.join("s3data/LAYOUT");
    std::fs::write(&marker, "tessaridb-s3 data layout 2\n").expect("remarked");
    let refused = cluster
        .restart_on(5, &current, &[])
        .await
        .expect_err("a newer layout is refused");
    assert!(
        refused.contains(
            "the data directory's layout is tessaridb-s3 data layout 2; this build reads layout 1"
        ),
        "{refused}"
    );
    assert_eq!(
        std::fs::read_to_string(&marker).expect("kept").trim(),
        "tessaridb-s3 data layout 2"
    );
}
