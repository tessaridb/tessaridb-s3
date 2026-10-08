//! The cluster's promises under faults: six S3 nodes RS(4, 2) over a three-node TessariDB cluster, with an S3 node
//! killed while a write or a read is in flight and the metadata leader killed under traffic. An acknowledged object
//! is never lost, and no object is ever visible with anything but the bytes that were written to it.

use std::time::{Duration, Instant};

use crate::disk::{first_shards, kill, writing};
use crate::docker;
use crate::meta::MetaCluster;
use crate::requests::{call, fetch, get_interrupted, put, put_interrupted};
use crate::s3::{Cluster, Meta, count, internal_tls};

/// How long the cluster may take to heal after its nodes come back: passes run every second.
const HEALED_WITHIN: Duration = Duration::from_secs(60);
/// How long the S3 nodes may take to acknowledge again once the metadata leader is gone.
const RECOVERED_WITHIN: Duration = Duration::from_secs(120);
const MIB: usize = 1024 * 1024;

/// `len` bytes that differ with `seed` and from stripe to stripe, so a block from another object or another stripe
/// cannot pass for the right one.
fn patterned(len: usize, seed: usize) -> Vec<u8> {
    (0..len)
        .map(|i| {
            let mixed = i
                .wrapping_mul(31)
                .wrapping_add(seed.wrapping_mul(7))
                .wrapping_add(i >> 20);
            u8::try_from(mixed % 251).expect("small")
        })
        .collect()
}

async fn healed(cluster: &Cluster) {
    let started = Instant::now();
    while count(&cluster.meta, "heals").await > 0 {
        assert!(
            started.elapsed() < HEALED_WITHIN,
            "still listed for healing after {HEALED_WITHIN:?}; see {}",
            cluster.logs.display()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The cluster's own nodes speak mutual TLS between them, as a deployment over a network would.
fn with_tls(meta: Meta) -> Meta {
    Meta {
        internal_tls: Some(internal_tls()),
        ..meta
    }
}

fn require_docker() {
    if let Err(why) = docker::available() {
        panic!("{why}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker and the tessaridb/tessaridb:0.33.2-beta image"]
async fn an_s3_node_killed_mid_write_or_mid_read_loses_nothing_acknowledged() {
    require_docker();
    let metadata = MetaCluster::start().await;
    let mut cluster = Cluster::start(with_tls(metadata.meta())).await;
    let log = std::fs::read_to_string(cluster.logs.join("n1.log")).expect("n1's log");
    assert!(
        log.contains("over mutual TLS"),
        "the internal surface runs over mutual TLS"
    );
    let front = 0;
    assert_eq!(
        call(&cluster.nodes[front], "PUT", "/faults", b"").await.0,
        200
    );

    // A node holding a shard dies once its part of the upload has begun: the write still reaches its quorum.
    let written = patterned(8 * MIB, 1);
    let victim = 3;
    let pid = cluster.nodes[victim].child.id().expect("running");
    let dir = cluster.nodes[victim].dir.clone();
    let status = put_interrupted(
        &cluster.nodes[front],
        "/faults/during-put",
        &written,
        2 * MIB,
        async {
            writing(&dir).await;
            kill(pid).await;
        },
    )
    .await
    .expect("the front node answers");
    assert_eq!(status, 200, "one shard lost is within the parity");
    assert_eq!(
        count(&cluster.meta, "heals").await,
        1,
        "the shard the dead node never finished is listed — the kill landed inside the write"
    );
    let (status, read) = fetch(&cluster.nodes[front], "/faults/during-put")
        .await
        .expect("the front node answers");
    assert_eq!(status, 200);
    assert!(read == written, "the acknowledged object reads back whole");

    // The node a client is writing through dies mid-body: nothing is acknowledged, and nothing becomes visible.
    let entry = 1;
    let pid = cluster.nodes[entry].child.id().expect("running");
    let dir = cluster.nodes[entry].dir.clone();
    let outcome = put_interrupted(
        &cluster.nodes[entry],
        "/faults/entry-killed",
        &patterned(8 * MIB, 2),
        2 * MIB,
        async {
            writing(&dir).await;
            kill(pid).await;
        },
    )
    .await;
    assert!(outcome.is_err(), "a dead node answered: {outcome:?}");
    assert_eq!(
        call(&cluster.nodes[front], "HEAD", "/faults/entry-killed", b"")
            .await
            .0,
        404,
        "an unacknowledged write is not visible elsewhere"
    );

    for down in [victim, entry] {
        cluster.nodes[down].child.wait().await.expect("reaped");
        cluster.restart(down).await;
    }
    healed(&cluster).await;

    // A node holding a shard the read needs dies while the read streams: the read goes on from the others, byte for
    // byte. Placement is per object, so the victim is a node other than the front one that received one of this
    // object's data shards.
    let before: Vec<_> = cluster
        .nodes
        .iter()
        .map(|node| first_shards(&node.dir))
        .collect();
    let large = patterned(24 * MIB, 3);
    let (status, body) = call(&cluster.nodes[front], "PUT", "/faults/during-get", &large).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        count(&cluster.meta, "heals").await,
        0,
        "every shard durable"
    );
    let victim = (0..cluster.nodes.len())
        .find(|&node| {
            node != front
                && first_shards(&cluster.nodes[node].dir)
                    .iter()
                    .any(|path| !before[node].contains(path))
        })
        .expect("another node holds one of this object's data shards");
    let pid = cluster.nodes[victim].child.id().expect("running");
    let (status, read) =
        get_interrupted(&cluster.nodes[front], "/faults/during-get", kill(pid)).await;
    assert_eq!(status, 200);
    assert_eq!(read.len(), large.len(), "the whole body arrived");
    assert!(read == large, "the body is the object's bytes");
    assert_eq!(
        count(&cluster.meta, "heals").await,
        1,
        "the read met the missing shard after the kill and listed it"
    );
}

/// What one PUT under the leader kill came to.
struct Attempt {
    key: String,
    body: Vec<u8>,
    acknowledged: bool,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker and the tessaridb/tessaridb:0.33.2-beta image"]
async fn the_metadata_leader_killed_under_traffic_loses_nothing_acknowledged() {
    require_docker();
    let metadata = MetaCluster::start().await;
    let cluster = Cluster::start(with_tls(metadata.meta())).await;
    assert_eq!(call(&cluster.nodes[0], "PUT", "/traffic", b"").await.0, 200);
    let leader = metadata.leader().await;

    let mut attempts: Vec<Attempt> = Vec::new();
    let mut killed_at = None;
    let mut acknowledged_after = 0_usize;
    for index in 0_usize.. {
        // Through every node in turn, so each one's metadata connections meet the failover.
        let node = &cluster.nodes[index % cluster.nodes.len()];
        let key = format!("/traffic/k{index:04}");
        let body = patterned(300 * 1024, index);
        let acknowledged = matches!(put(node, &key, &body).await, Ok((200, _)));
        attempts.push(Attempt {
            key,
            body,
            acknowledged,
        });
        let acknowledged_total = attempts
            .iter()
            .filter(|attempt| attempt.acknowledged)
            .count();
        match killed_at {
            None if acknowledged_total >= 5 => {
                metadata.kill(leader).await;
                killed_at = Some(Instant::now());
            }
            None => {}
            Some(at) => {
                acknowledged_after = acknowledged_after.saturating_add(usize::from(acknowledged));
                if acknowledged_after >= 5 {
                    break;
                }
                assert!(
                    at.elapsed() < RECOVERED_WITHIN,
                    "S3 acknowledged {acknowledged_after} write(s) in {RECOVERED_WITHIN:?} after the leader died; \
                     see {}\n{}",
                    cluster.logs.display(),
                    metadata
                        .said(leader.saturating_add(1).checked_rem(3).unwrap_or_default())
                        .await
                );
            }
        }
    }

    let unacknowledged = attempts
        .iter()
        .filter(|attempt| !attempt.acknowledged)
        .count();
    eprintln!(
        "leader n{leader} killed; {} writes, {unacknowledged} unacknowledged, the fifth acknowledged after the kill at {:?}",
        attempts.len(),
        killed_at.map(|at| at.elapsed())
    );
    let survivors = metadata.leaders().await;
    assert!(
        survivors.len() == 1 && survivors[0] != leader,
        "a survivor leads instead of the killed node: {survivors:?}"
    );
    let reader = &cluster.nodes[0];
    for attempt in &attempts {
        let (status, read) = fetch(reader, &attempt.key)
            .await
            .expect("the reader answers");
        if attempt.acknowledged {
            assert_eq!(status, 200, "{} was acknowledged and is gone", attempt.key);
            assert!(read == attempt.body, "{} reads back whole", attempt.key);
        } else {
            assert!(
                status == 404 || (status == 200 && read == attempt.body),
                "{} was not acknowledged and is visible as something else: {status}",
                attempt.key
            );
        }
    }
}
