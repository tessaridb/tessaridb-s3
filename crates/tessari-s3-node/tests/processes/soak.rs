//! A long run of mixed traffic on one 4+2 cluster: writes, reads, overwrites, deletes and listings for
//! `TESSARIDB_S3_SOAK_SECS` (it runs only when that is set; 7200 is the two-hour soak), with each node's resident
//! memory and open descriptors sampled every minute. At the end every acknowledged object reads back as last written,
//! every deleted one is gone, the heal queue is empty, and the samples are printed for the soak report. Growth is
//! judged in the report, not asserted here: a trend needs the whole series.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use futures_util::future::join_all;

use crate::meta_from_env;
use crate::requests::{call, fetch, put};
use crate::s3::{Cluster, Node, count};

/// Workers sending traffic.
const WORKERS: usize = 8;
/// Keys each worker cycles through, so objects are overwritten and deleted many times.
const KEYS_PER_WORKER: usize = 32;

/// `len` bytes that differ with `seed`.
fn bytes(len: usize, seed: u32) -> Vec<u8> {
    (0..len)
        .map(|i| {
            u8::try_from(
                u32::try_from(i)
                    .unwrap_or(0)
                    .wrapping_add(seed)
                    .checked_rem(251)
                    .unwrap_or(0),
            )
            .expect("small")
        })
        .collect()
}

/// Resident memory in KiB and open descriptors of process `pid`, as `ps` and `lsof` report them.
async fn sample(pid: u32) -> (u64, usize) {
    let rss = tokio::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .await
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let fds = tokio::process::Command::new("lsof")
        .args(["-n", "-P", "-p", &pid.to_string()])
        .output()
        .await
        .map(|out| {
            out.stdout
                .iter()
                .filter(|byte| **byte == b'\n')
                .count()
                .saturating_sub(1)
        })
        .unwrap_or(0);
    (rss, fds)
}

/// One worker until `deadline`: what it last left at each of its keys (`None` when deleted), and its error count.
async fn work(
    node: &Node,
    worker: usize,
    deadline: Instant,
) -> (BTreeMap<String, Option<u32>>, usize) {
    let mut last = BTreeMap::new();
    let mut errors = 0_usize;
    let mut turn = 0_u32;
    while Instant::now() < deadline {
        let slot = usize::try_from(turn)
            .unwrap_or(0)
            .checked_rem(KEYS_PER_WORKER)
            .unwrap_or(0);
        let key = format!("/soak/w{worker}/k{slot}");
        let seed = turn
            .wrapping_mul(31)
            .wrapping_add(u32::try_from(worker).unwrap_or(0));
        let size = [4 << 10, 256 << 10, 2 << 20][usize::try_from(turn)
            .unwrap_or(0)
            .checked_rem(3)
            .unwrap_or(0)];
        match turn.checked_rem(10).unwrap_or(0) {
            // One in ten turns deletes the key; the others overwrite it and read it back.
            9 => match call(node, "DELETE", &key, b"").await {
                (204, _) => {
                    last.insert(key, None);
                }
                (status, body) => {
                    eprintln!("soak error: DELETE {key} answered {status}: {body}");
                    errors = errors.saturating_add(1);
                }
            },
            _ => match put(node, &key, &bytes(size, seed)).await {
                Ok((200, _)) => {
                    last.insert(key.clone(), Some(seed));
                    match fetch(node, &key).await {
                        Ok((200, read)) if read == bytes(size, seed) => {}
                        other => {
                            let status = other.map(|(status, _)| status);
                            eprintln!("soak error: GET {key} after its PUT answered {status:?}");
                            errors = errors.saturating_add(1);
                        }
                    }
                }
                // Not acknowledged: what was there before may or may not have been replaced.
                other => {
                    eprintln!("soak error: PUT {key} answered {other:?}");
                    last.remove(&key);
                    errors = errors.saturating_add(1);
                }
            },
        }
        turn = turn.wrapping_add(1);
    }
    (last, errors)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a soak of hours: set TESSARIDB_S3_TEST_META, _USER, _PASSWORD and TESSARIDB_S3_SOAK_SECS"]
async fn mixed_traffic_for_hours_loses_nothing_and_drains_its_queues() {
    let _quiet = crate::QUIET.lock().await;
    // Hours long, so it runs only when asked for: a plain run of the process tests leaves it out.
    let Some(secs) = std::env::var("TESSARIDB_S3_SOAK_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    else {
        eprintln!("soak not run: set TESSARIDB_S3_SOAK_SECS (7200 for the two-hour soak)");
        return;
    };
    let cluster = Cluster::start(meta_from_env()).await;
    assert_eq!(call(&cluster.nodes[0], "PUT", "/soak", b"").await.0, 200);
    let started = Instant::now();
    let deadline = started
        .checked_add(Duration::from_secs(secs))
        .expect("a deadline in range");
    let sampling = async {
        let mut minute = 0_u64;
        while Instant::now() < deadline {
            let mut line = format!("soak sample t={minute}m");
            for node in &cluster.nodes {
                let (rss, fds) = sample(node.child.id().unwrap_or(0)).await;
                line.push_str(&format!(" {}:{}KiB/{}fd", node.name, rss, fds));
            }
            eprintln!("{line}");
            minute = minute.saturating_add(1);
            let left = deadline.saturating_duration_since(Instant::now());
            tokio::time::sleep(left.min(Duration::from_secs(60))).await;
        }
    };
    let workers = join_all((0..WORKERS).map(|worker| {
        let node = &cluster.nodes[worker.checked_rem(cluster.nodes.len()).unwrap_or(0)];
        async move { work(node, worker, deadline).await }
    }));
    let ((), results) = tokio::join!(sampling, workers);
    let mut errors = 0_usize;
    let mut checked = 0_usize;
    for (worker, (last, worker_errors)) in results.into_iter().enumerate() {
        errors = errors.saturating_add(worker_errors);
        let reader = &cluster.nodes[worker
            .saturating_add(1)
            .checked_rem(cluster.nodes.len())
            .unwrap_or(0)];
        for (key, state) in last {
            let (status, read) = fetch(reader, &key).await.expect("the node answers");
            match state {
                Some(seed) => {
                    assert_eq!(status, 200, "{key} acknowledged and lost");
                    let sizes = [4 << 10, 256 << 10, 2 << 20];
                    assert!(
                        sizes.iter().any(|size| read == bytes(*size, seed)),
                        "{key} reads back as last written"
                    );
                }
                None => assert_eq!(status, 404, "{key} deleted and still there"),
            }
            checked = checked.saturating_add(1);
        }
    }
    let quiet = Instant::now();
    while count(&cluster.meta, "heal_queue").await > 0 {
        assert!(
            quiet.elapsed() < Duration::from_secs(120),
            "the heal queue did not drain"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    eprintln!(
        "soak done: {secs} s, {checked} keys checked, {errors} requests not acknowledged or misread, heal queue empty"
    );
    assert_eq!(
        errors, 0,
        "every request was acknowledged and read back whole"
    );
}
