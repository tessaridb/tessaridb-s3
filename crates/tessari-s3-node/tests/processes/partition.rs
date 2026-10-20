//! One node cut off from the others while it keeps running: its internal surface is reached through a proxy that,
//! while the partition lasts, accepts connections and swallows every byte in both directions, so its peers see a node
//! that answers nothing rather than one that refuses. Writes through the other nodes are acknowledged on the shards
//! they can place and read back whole; once the partition ends the cut-off node is healed to hold its shards again.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::requests::{call, fetch, put};
use crate::s3::{Cluster, count, shard_files};
use crate::{HEALED_WITHIN, large, meta_from_env};

/// Copies `from` to `to` while `cut` is false and drops what it reads while it is true.
async fn forward(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    cut: Arc<AtomicBool>,
) {
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = match from.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        if !cut.load(Ordering::SeqCst) && to.write_all(&buffer[..read]).await.is_err() {
            return;
        }
    }
}

/// A proxy to `target` on a free local port; `cut` set means the partition is on.
async fn proxy(target: String, cut: Arc<AtomicBool>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("proxy port");
    let address = listener.local_addr().expect("bound").to_string();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let cut = Arc::clone(&cut);
            let target = target.clone();
            tokio::spawn(async move {
                let Ok(server) = TcpStream::connect(&target).await else {
                    return;
                };
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                tokio::join!(
                    forward(client_read, server_write, Arc::clone(&cut)),
                    forward(server_read, client_write, cut),
                );
            });
        }
    });
    address
}

#[tokio::test]
#[ignore = "needs TESSARIDB_S3_TEST_META, _USER and _PASSWORD and the built binary"]
async fn a_node_cut_off_from_its_peers_loses_nothing_and_is_healed_after() {
    let _quiet = crate::QUIET.lock().await;
    let mut cluster = Cluster::start(meta_from_env()).await;
    let cut_off = cluster.nodes.len().saturating_sub(1);
    let cut = Arc::new(AtomicBool::new(false));
    let through = proxy(cluster.nodes[cut_off].internal.clone(), Arc::clone(&cut)).await;
    // The node registers its proxy as the address its peers reach it at.
    crate::disk::kill(cluster.nodes[cut_off].child.id().expect("running")).await;
    let _reaped = cluster.nodes[cut_off].child.wait().await;
    cluster
        .restart_with(cut_off, &[("TESSARIDB_S3_INTERNAL_ADVERTISE", &through)])
        .await;
    let front = &cluster.nodes[0];
    assert_eq!(call(front, "PUT", "/cut", b"").await.0, 200);
    let object = large();
    let (status, body) = put(front, "/cut/before", &object).await.expect("answered");
    assert_eq!(status, 200, "{body}");
    let held_before = shard_files(&cluster.nodes[cut_off].dir);

    cut.store(true, Ordering::SeqCst);
    let mut written = Vec::new();
    for key in ["/cut/during-1", "/cut/during-2"] {
        let started = Instant::now();
        let (status, body) = put(front, key, &object).await.expect("answered");
        assert_eq!(status, 200, "{key}: {body}");
        eprintln!(
            "{key} acknowledged in {:?} while partitioned",
            started.elapsed()
        );
        written.push(key);
    }
    for key in written.iter().copied().chain(["/cut/before"]) {
        let (status, read) = fetch(&cluster.nodes[1], key).await.expect("answered");
        assert_eq!(status, 200, "{key}");
        assert!(read == object, "{key} reads back whole while partitioned");
    }
    assert_eq!(
        shard_files(&cluster.nodes[cut_off].dir),
        held_before,
        "nothing reached the cut-off node"
    );

    cut.store(false, Ordering::SeqCst);
    let started = Instant::now();
    while shard_files(&cluster.nodes[cut_off].dir) < held_before.saturating_add(written.len())
        || count(&cluster.meta, "heal_queue").await > 0
    {
        assert!(
            started.elapsed() < HEALED_WITHIN.saturating_mul(3),
            "not healed after the partition; see {}",
            cluster.logs.display()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    eprintln!("healed {:?} after the partition ended", started.elapsed());
    for key in written.iter().copied().chain(["/cut/before"]) {
        let (status, read) = fetch(&cluster.nodes[cut_off], key).await.expect("answered");
        assert_eq!(status, 200, "{key}");
        assert!(
            read == object,
            "{key} reads back whole through the healed node"
        );
    }
}
