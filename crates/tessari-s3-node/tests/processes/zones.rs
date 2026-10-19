//! A whole failure domain lost under a running writer: nine S3 nodes RS(4, 2) in three zones of three, registering zone
//! by zone, so a layout blind to zones would take six nodes from the first two zones and three shards of every object
//! from one; the layout takes at most two from each — the parity. Killing a zone mid-run leaves every acknowledged
//! object readable byte for byte, every write started afterwards acknowledged at the write quorum, and once the zone
//! returns the shards it missed are healed.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::disk::kill;
use crate::faults::{MIB, healed, patterned, require_docker};
use crate::meta::MetaCluster;
use crate::requests::{call, fetch};
use crate::s3::{Cluster, Meta, count};

const WRITES: usize = 24;
/// Writes acknowledged before the zone goes down.
const BEFORE_KILL: usize = 8;
/// Each node's zone, `n1` first: the first two zones fill before the third registers.
const ZONES: [&str; 9] = ["a", "a", "a", "b", "b", "b", "c", "c", "c"];
/// Zone `a`: `n1`..`n3`, every node of which registered before a layout could be made.
const ZONE_A: [usize; 3] = [0, 1, 2];
/// The node the client talks to, in zone `c`.
const FRONT: usize = 8;

/// One PUT of the run: its key and body, whether it started after the zone was down, and its status.
struct Write {
    key: String,
    body: Vec<u8>,
    after_kill: bool,
    status: u16,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker and the tessaridb/tessaridb:0.36.2 image"]
async fn a_zone_killed_under_a_writer_loses_nothing_acknowledged_and_heals_when_it_returns() {
    require_docker();
    let metadata = MetaCluster::start().await;
    let mut cluster = Cluster::start(Meta {
        zones: Some(&ZONES),
        ..metadata.meta()
    })
    .await;
    let front = &cluster.nodes[FRONT];
    assert_eq!(call(front, "PUT", "/zones", b"").await.0, 200);

    let acknowledged = AtomicUsize::new(0);
    let down = AtomicBool::new(false);
    let writer = async {
        let mut writes = Vec::with_capacity(WRITES);
        for n in 0..WRITES {
            let key = format!("/zones/w{n}");
            let body = patterned(3 * MIB, n);
            let after_kill = down.load(Ordering::Acquire);
            let status = call(front, "PUT", &key, &body).await.0;
            if status == 200 {
                acknowledged.fetch_add(1, Ordering::Release);
            }
            writes.push(Write {
                key,
                body,
                after_kill,
                status,
            });
        }
        writes
    };
    let pids: Vec<u32> = ZONE_A
        .iter()
        .map(|&index| cluster.nodes[index].child.id().expect("running"))
        .collect();
    let killer = async {
        while acknowledged.load(Ordering::Acquire) < BEFORE_KILL {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        for &pid in &pids {
            kill(pid).await;
        }
        down.store(true, Ordering::Release);
    };
    let (writes, ()) = tokio::join!(writer, killer);

    let late: Vec<&Write> = writes.iter().filter(|write| write.after_kill).collect();
    assert!(!late.is_empty(), "writes went on after the zone was down");
    for write in &late {
        assert_eq!(
            write.status,
            200,
            "{} with zone a down; see {}",
            write.key,
            cluster.logs.display()
        );
    }
    assert!(
        count(&cluster.meta, "heal_queue").await >= late.len(),
        "each write after the kill lacks zone a's shards and is listed"
    );
    for write in writes.iter().filter(|write| write.status == 200) {
        let (status, read) = fetch(front, &write.key)
            .await
            .expect("the front node answers");
        assert_eq!(status, 200, "{}", write.key);
        assert!(
            read == write.body,
            "{} reads back byte for byte with zone a down",
            write.key
        );
    }

    for index in ZONE_A {
        cluster.nodes[index].child.wait().await.expect("reaped");
        cluster.restart(index).await;
    }
    healed(&cluster).await;
    let front = &cluster.nodes[FRONT];
    for write in writes.iter().filter(|write| write.status == 200) {
        let (status, read) = fetch(front, &write.key)
            .await
            .expect("the front node answers");
        assert_eq!(status, 200, "{}", write.key);
        assert!(
            read == write.body,
            "{} reads back byte for byte after healing",
            write.key
        );
    }
}
