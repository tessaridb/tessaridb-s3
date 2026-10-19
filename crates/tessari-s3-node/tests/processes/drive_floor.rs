//! Nodes whose data drive is below its free-space floor: each refuses the shards it would store and keeps serving
//! reads. With m nodes refusing, a write still reaches k durable shards; with one more it is refused `503`, and
//! nothing is acknowledged. Once the floor is lifted the shards the refusing nodes missed are healed back.

use std::time::{Duration, Instant};

use crate::requests::{call, fetch};
use crate::s3::{Cluster, count};
use crate::{HEALED_WITHIN, large, meta_from_env};

/// A floor no drive can meet: every new data file is refused.
const UNREACHABLE: &str = "18446744073709551615";

#[tokio::test]
#[ignore = "needs TESSARIDB_S3_TEST_META, _USER and _PASSWORD and the built binary"]
async fn nodes_below_their_floor_refuse_shards_and_keep_serving_reads() {
    let mut cluster = Cluster::start(meta_from_env()).await;
    assert_eq!(call(&cluster.nodes[0], "PUT", "/floor", b"").await.0, 200);
    let object = large();
    let (status, body) = call(&cluster.nodes[0], "PUT", "/floor/before", &object).await;
    assert_eq!(status, 200, "{body}");

    let last = cluster.nodes.len().saturating_sub(1);
    for index in [last, last.saturating_sub(1)] {
        crate::disk::kill(cluster.nodes[index].child.id().expect("running")).await;
        let _reaped = cluster.nodes[index].child.wait().await;
        cluster
            .restart_with(index, &[("TESSARIDB_S3_DATA_MIN_FREE_BYTES", UNREACHABLE)])
            .await;
    }
    let (status, body) = call(&cluster.nodes[0], "PUT", "/floor/two-full", &object).await;
    assert_eq!(
        status, 200,
        "k shards durable with m nodes refusing: {body}"
    );

    let third = last.saturating_sub(2);
    crate::disk::kill(cluster.nodes[third].child.id().expect("running")).await;
    let _reaped = cluster.nodes[third].child.wait().await;
    cluster
        .restart_with(third, &[("TESSARIDB_S3_DATA_MIN_FREE_BYTES", UNREACHABLE)])
        .await;
    let (status, body) = call(&cluster.nodes[0], "PUT", "/floor/three-full", &object).await;
    assert_eq!(status, 503, "fewer than k shards durable: {body}");
    assert!(body.contains("ServiceUnavailable"), "{body}");
    let (status, _) = fetch(&cluster.nodes[0], "/floor/three-full")
        .await
        .expect("the node answers");
    assert_eq!(status, 404, "a refused write is not acknowledged");
    for (key, reader) in [("/floor/before", last), ("/floor/two-full", third)] {
        let (status, read) = fetch(&cluster.nodes[reader], key)
            .await
            .expect("the node answers");
        assert_eq!(status, 200, "{key} read through a node below its floor");
        assert!(read == object, "{key} reads back whole");
    }

    for index in [third, last.saturating_sub(1), last] {
        crate::disk::kill(cluster.nodes[index].child.id().expect("running")).await;
        let _reaped = cluster.nodes[index].child.wait().await;
        cluster.restart(index).await;
    }
    let started = Instant::now();
    while count(&cluster.meta, "heal_queue").await > 0 {
        assert!(
            started.elapsed() < HEALED_WITHIN,
            "the missed shards were not healed; see {}",
            cluster.logs.display()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let (status, body) = call(&cluster.nodes[0], "PUT", "/floor/after", &object).await;
    assert_eq!(status, 200, "writes are taken again: {body}");
}
