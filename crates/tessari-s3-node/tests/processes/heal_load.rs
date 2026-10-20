//! Healing while clients work: a node is stopped, objects are written without it, and once it is back its shards are
//! healed while readers and writers keep the other nodes busy. The backlog must drain under that load with every read
//! answered whole; then sixteen 64 MiB objects are read at once, the load that once ended long shard responses early.
//! The timings are printed for the bench report; the test asserts outcomes, never speeds.

use std::time::{Duration, Instant};

use futures_util::future::join_all;

use crate::meta_from_env;
use crate::requests::{call, fetch, put};
use crate::s3::Node;
use crate::s3::{Cluster, count};

/// How long the backlog may take to drain under load.
const DRAINED_WITHIN: Duration = Duration::from_secs(300);
/// Readers and writers kept busy while the backlog drains.
const WORKERS: usize = 8;

/// `len` bytes that differ with `seed`.
fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 251).expect("small").wrapping_add(seed))
        .collect()
}

/// The sample at `permille` of `samples` in order, in milliseconds.
fn quantile(samples: &mut [Duration], permille: usize) -> f64 {
    samples.sort_unstable();
    let last = samples.len().saturating_sub(1);
    let index = last.saturating_mul(permille).saturating_add(500) / 1000;
    samples
        .get(index)
        .map_or(0.0, |sample| sample.as_secs_f64() * 1000.0)
}

/// One worker: reads the kept objects in turn and writes a small one between reads, until `until` says stop.
async fn work(
    node: &Node,
    worker: usize,
    kept: &[(String, Vec<u8>)],
    until: &tokio::sync::watch::Receiver<bool>,
) -> (Vec<Duration>, Vec<Duration>) {
    let (mut reads, mut writes) = (Vec::new(), Vec::new());
    let small = bytes(
        64 * 1024,
        u8::try_from(worker.checked_rem(200).unwrap_or(0)).expect("small"),
    );
    let mut turn = 0_usize;
    while !*until.borrow() {
        let (key, expected) = &kept[worker
            .saturating_add(turn)
            .checked_rem(kept.len())
            .unwrap_or(0)];
        let started = Instant::now();
        let (status, read) = fetch(node, key).await.expect("the node answers");
        reads.push(started.elapsed());
        assert_eq!(status, 200, "{key}");
        assert!(read == *expected, "{key} reads back whole while healing");
        let started = Instant::now();
        let (status, body) = put(node, &format!("/heal/w{worker}-{turn}"), &small)
            .await
            .expect("the node answers");
        writes.push(started.elapsed());
        assert_eq!(status, 200, "{body}");
        turn = turn.saturating_add(1);
    }
    (reads, writes)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs TESSARIDB_S3_TEST_META, _USER and _PASSWORD and the built binary"]
async fn a_returning_node_is_healed_while_clients_read_and_write() {
    let _quiet = crate::QUIET.lock().await;
    let mut cluster = Cluster::start(meta_from_env()).await;
    assert_eq!(call(&cluster.nodes[0], "PUT", "/heal", b"").await.0, 200);
    let kept: Vec<(String, Vec<u8>)> = (0..16_u8)
        .map(|n| (format!("/heal/kept-{n}"), bytes(1 << 20, n)))
        .collect();
    for (key, object) in &kept {
        assert_eq!(
            put(&cluster.nodes[0], key, object)
                .await
                .expect("answered")
                .0,
            200
        );
    }

    let down = cluster.nodes.len().saturating_sub(1);
    crate::disk::kill(cluster.nodes[down].child.id().expect("running")).await;
    let _reaped = cluster.nodes[down].child.wait().await;
    let missed = bytes(4 << 20, 7);
    for n in 0..40 {
        let (status, body) = put(&cluster.nodes[0], &format!("/heal/missed-{n}"), &missed)
            .await
            .expect("answered");
        assert_eq!(status, 200, "{body}");
    }
    let backlog = count(&cluster.meta, "heal_queue").await;
    assert!(
        backlog >= 40,
        "every write without the node is listed: {backlog}"
    );

    cluster.restart(down).await;
    let (stop, until) = tokio::sync::watch::channel(false);
    let started = Instant::now();
    let workers = join_all((0..WORKERS).map(|worker| {
        let node = &cluster.nodes[worker.checked_rem(down).unwrap_or(0)];
        let kept = &kept;
        let until = until.clone();
        async move { work(node, worker, kept, &until).await }
    }));
    let drained = async {
        while count(&cluster.meta, "heal_queue").await > 0 {
            assert!(
                started.elapsed() < DRAINED_WITHIN,
                "the backlog did not drain under load; see {}",
                cluster.logs.display()
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let took = started.elapsed();
        stop.send(true).expect("workers listening");
        took
    };
    let (results, took) = tokio::join!(workers, drained);
    let (mut reads, mut writes): (Vec<Duration>, Vec<Duration>) = results.into_iter().fold(
        (Vec::new(), Vec::new()),
        |(mut reads, mut writes), (read, write)| {
            reads.extend(read);
            writes.extend(write);
            (reads, writes)
        },
    );
    assert!(
        !reads.is_empty() && !writes.is_empty(),
        "clients worked while it healed"
    );
    eprintln!(
        "heal under load: backlog {backlog} ids drained in {took:?} with {WORKERS} workers; {} reads p50 {:.1} ms p99 {:.1} ms; {} writes p50 {:.1} ms p99 {:.1} ms",
        reads.len(),
        quantile(&mut reads, 500),
        quantile(&mut reads, 990),
        writes.len(),
        quantile(&mut writes, 500),
        quantile(&mut writes, 990),
    );

    // Sixteen 64 MiB reads at once, each answered whole.
    let huge = bytes(64 << 20, 3);
    assert_eq!(
        put(&cluster.nodes[0], "/heal/huge", &huge)
            .await
            .expect("answered")
            .0,
        200
    );
    let started = Instant::now();
    let huge = &huge;
    let reads = join_all((0..16_usize).map(|n| {
        let node = &cluster.nodes[n.checked_rem(cluster.nodes.len()).unwrap_or(0)];
        async move {
            let began = Instant::now();
            let (status, read) = fetch(node, "/heal/huge").await.expect("the node answers");
            (status, read == *huge, began.elapsed())
        }
    }))
    .await;
    let mut times = Vec::new();
    for (status, whole, took) in reads {
        assert_eq!(status, 200);
        assert!(whole, "a 64 MiB read answered whole");
        times.push(took);
    }
    eprintln!(
        "16 concurrent 64 MiB reads in {:?}: p50 {:.0} ms p99 {:.0} ms",
        started.elapsed(),
        quantile(&mut times, 500),
        quantile(&mut times, 990),
    );
}
