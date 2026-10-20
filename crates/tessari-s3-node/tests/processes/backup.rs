//! The two-half backup (G024 C2), online and in the order that makes the halves agree: the metadata snapshot FIRST,
//! then every node's data directory, within the reclaim grace. Bytes are durable before the commit that names them and
//! a freed file waits out the grace, so every object the snapshot names has its bytes in the copy — including one
//! deleted or overwritten after the snapshot — and what was written after it is an orphan the restore reclaims. The
//! drill takes it under writes, restores both halves onto a fresh metadata node and the same six node identities,
//! times both, and reads every object the snapshot names back byte for byte.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};

use crate::faults::patterned;
use crate::requests::{call, fetch, put_to};
use crate::s3::{Cluster, Meta, free_port, scratch};
use crate::{QUIET, need};

const OWNER: &str = "owner";

/// A metadata node of the engine at `engine`, serving `store` on a free port; killed when dropped.
struct MetaNode {
    _child: Child,
    address: String,
}

async fn serve_meta(engine: &Path, store: &Path, password: &str) -> MetaNode {
    let address = format!("127.0.0.1:{}", free_port());
    let http = format!("127.0.0.1:{}", free_port());
    let child = Command::new(engine)
        .arg(store)
        .args(["--serve", &address, "--http", &http])
        .env("TESSARIDB_INITIAL_USER", OWNER)
        .env("TESSARIDB_INITIAL_PASSWORD", password)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("the engine starts");
    let started = Instant::now();
    while tokio::net::TcpStream::connect(&address).await.is_err() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the engine never served"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    MetaNode {
        _child: child,
        address,
    }
}

fn meta(address: &str, password: &str) -> Meta {
    Meta {
        addresses: address.to_owned(),
        user: OWNER.to_owned(),
        password: password.to_owned(),
        replication: None,
        notify_targets: None,
        zones: None,
    }
}

/// `engine` with `args`, run to completion; its failure fails the test with its own words.
async fn run(engine: &Path, args: &[&str], password: &str) {
    let output = Command::new(engine)
        .args(args)
        .env("TESSARIDB_PASSWORD", password)
        .output()
        .await
        .expect("the engine runs");
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Copies the directory tree `from` to `to`.
async fn copy_tree(from: &Path, to: &Path) {
    let status = Command::new("cp")
        .arg("-R")
        .arg(from)
        .arg(to)
        .status()
        .await
        .expect("cp runs");
    assert!(status.success(), "copied {}", from.display());
}

/// Each object acknowledged, with the moment it was, until `stop` is set.
async fn write_until(hosts: Vec<String>, stop: Arc<AtomicBool>) -> Vec<(usize, Instant)> {
    let mut acknowledged = Vec::new();
    let mut index = 1000_usize;
    while !stop.load(Ordering::Acquire) {
        let host = &hosts[index.checked_rem(hosts.len()).unwrap_or_default()];
        if let Ok(200) = put_to(
            host,
            &format!("/kept/o{index}"),
            &patterned(64 * 1024, index),
        )
        .await
        {
            acknowledged.push((index, Instant::now()));
        }
        index = index.saturating_add(1);
    }
    acknowledged
}

#[tokio::test]
#[ignore = "needs TESSARIDB_S3_TEST_ENGINE (the tessaridb binary the metadata runs on)"]
async fn an_online_backup_restores_every_object_the_snapshot_names() {
    let _quiet = QUIET.lock().await;
    let engine = PathBuf::from(need("TESSARIDB_S3_TEST_ENGINE"));
    let password = uuid::Uuid::new_v4().simple().to_string();
    let work = scratch("backup");
    std::fs::create_dir_all(&work).expect("work directory");
    let first = serve_meta(&engine, &work.join("meta-a"), &password).await;
    let mut cluster = Cluster::start(meta(&first.address, &password)).await;
    let (status, body) = call(&cluster.nodes[0], "PUT", "/kept", b"").await;
    assert_eq!(status, 200, "{body}");
    // Objects before the backup: small ones and ones spanning several erasure blocks.
    let before: Vec<(usize, usize)> = (0..24)
        .map(|index| (index, if index % 3 == 0 { 3 << 20 } else { 64 * 1024 }))
        .collect();
    for (index, len) in &before {
        let (status, body) = call(
            &cluster.nodes[index % 6],
            "PUT",
            &format!("/kept/o{index}"),
            &patterned(*len, *index),
        )
        .await;
        assert_eq!(status, 200, "{body}");
    }
    let hosts: Vec<String> = cluster.nodes.iter().map(|node| node.s3.clone()).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let writer = tokio::spawn(write_until(hosts, Arc::clone(&stop)));
    tokio::time::sleep(Duration::from_secs(1)).await;

    // The backup, online: the metadata snapshot first.
    let backup_started = Instant::now();
    let snapshot = work.join("meta.snapshot");
    let snapshot_text = snapshot.display().to_string();
    run(
        &engine,
        &[
            "--at",
            &first.address,
            "--user",
            OWNER,
            "--backup",
            &snapshot_text,
        ],
        &password,
    )
    .await;
    let snapshot_taken = Instant::now();
    // After the snapshot: some objects deleted and some overwritten — their old bytes must still be in the copy.
    for index in [1_usize, 4, 7] {
        let (status, _) = call(&cluster.nodes[0], "DELETE", &format!("/kept/o{index}"), b"").await;
        assert_eq!(status, 204);
    }
    for index in [2_usize, 3, 5] {
        let (status, _) = call(
            &cluster.nodes[1],
            "PUT",
            &format!("/kept/o{index}"),
            b"later",
        )
        .await;
        assert_eq!(status, 200);
    }
    // Then every node's data directory.
    for node in &cluster.nodes {
        copy_tree(&node.dir, &work.join(format!("data-{}", node.name))).await;
    }
    let backup_took = backup_started.elapsed();
    stop.store(true, Ordering::Release);
    let during = writer.await.expect("the writer finished");

    // The restore: both halves back, on a fresh metadata node and the same six identities and drives.
    for index in 0..cluster.nodes.len() {
        cluster.stop(index).await;
    }
    drop(first);
    let restore_started = Instant::now();
    let restored_store = work.join("meta-b");
    let restored_text = restored_store.display().to_string();
    run(
        &engine,
        &[&restored_text, "--restore", &snapshot_text],
        &password,
    )
    .await;
    let second = serve_meta(&engine, &restored_store, &password).await;
    for node in &cluster.nodes {
        std::fs::remove_dir_all(&node.dir).expect("the drive is replaced");
        copy_tree(&work.join(format!("data-{}", node.name)), &node.dir).await;
    }
    cluster.reopen_on(meta(&second.address, &password)).await;
    let restore_took = restore_started.elapsed();

    let named: Vec<(usize, usize)> = before
        .iter()
        .copied()
        .chain(
            during
                .iter()
                .filter(|(_, at)| *at < backup_started)
                .map(|(index, _)| (*index, 64 * 1024)),
        )
        .collect();
    let mut wrong = Vec::new();
    for (position, (index, len)) in named.iter().enumerate() {
        let node = &cluster.nodes[position % 6];
        match fetch(node, &format!("/kept/o{index}")).await {
            Ok((200, bytes)) if bytes == patterned(*len, *index) => {}
            other => wrong.push(format!(
                "o{index}: {:?}",
                other.map(|(status, bytes)| (status, bytes.len()))
            )),
        }
    }
    // Every object the restored metadata lists is served whole: no record without its bytes.
    let (status, listing) = call(&cluster.nodes[0], "GET", "/kept", b"").await;
    assert_eq!(status, 200, "{listing}");
    let listed: Vec<usize> = listing
        .split("<Key>o")
        .skip(1)
        .filter_map(|rest| rest.split('<').next()?.parse().ok())
        .collect();
    assert!(
        listed.len() >= named.len(),
        "listed {} of {}",
        listed.len(),
        named.len()
    );
    for index in &listed {
        let len = before
            .iter()
            .find(|(at, _)| at == index)
            .map_or(64 * 1024, |(_, len)| *len);
        match fetch(&cluster.nodes[index % 6], &format!("/kept/o{index}")).await {
            Ok((200, bytes)) if bytes == patterned(len, *index) => {}
            other => wrong.push(format!(
                "listed o{index}: {:?}",
                other.map(|(status, bytes)| (status, bytes.len()))
            )),
        }
    }
    // Written after the snapshot began: absent, or whole — never anything else.
    let mut later_present = 0_usize;
    for (index, at) in &during {
        if *at < snapshot_taken {
            continue;
        }
        let node = &cluster.nodes[index % 6];
        match fetch(node, &format!("/kept/o{index}")).await {
            Ok((404, _)) => {}
            Ok((200, bytes)) if bytes == patterned(64 * 1024, *index) => {
                later_present = later_present.saturating_add(1);
            }
            other => wrong.push(format!(
                "later o{index}: {:?}",
                other.map(|(status, bytes)| (status, bytes.len()))
            )),
        }
    }
    println!(
        "backup: named={} | read back={} | written during={} (present after restore {later_present}) | backup took {backup_took:?} | restore took {restore_took:?}",
        named.len(),
        named.len().saturating_sub(wrong.len()),
        during.len()
    );
    assert!(
        wrong.is_empty(),
        "{wrong:?}; see {}",
        cluster.logs.display()
    );
    drop(second);
    std::fs::remove_dir_all(&work).ok();
}
