//! Reading erasure-coded objects back on the RS(4, 2) cluster of the `erasure` module: whole and by range across a
//! stripe boundary, with shards missing or damaged up to the parity — each such read listing the object for healing
//! — and refused with 503 when fewer than four shards remain.

use std::path::{Path, PathBuf};

use crate::erasure::{Member, count, member};
use crate::{IGNORED, call, call_with};

/// 2.5 MiB that differ from stripe to stripe: three stripes, the last one short.
fn striped() -> Vec<u8> {
    (0..2_621_440_u32)
        .map(|i| u8::try_from(i.wrapping_mul(31).wrapping_add(i >> 20) % 251).expect("small"))
        .collect()
}

/// The file of shard `index` on whichever node holds it.
fn shard_file(member: &Member, index: u16) -> PathBuf {
    let suffix = format!(".s{index}");
    std::iter::once(&member.dir)
        .chain(member.peers.iter().map(|peer| &peer.dir))
        .find_map(|dir| find(dir, &suffix))
        .unwrap_or_else(|| panic!("shard {index} is on some node"))
}

fn find(dir: &Path, suffix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        if path.is_dir() {
            find(&path, suffix)
        } else {
            path.to_string_lossy().ends_with(suffix).then_some(path)
        }
    })
}

/// A cluster member holding `striped()` at `/shards/big`.
async fn written() -> Member {
    let member = member().await;
    assert_eq!(
        call(&member.state, "PUT", "/shards", vec![], b"")
            .await
            .status,
        200
    );
    let put = call(&member.state, "PUT", "/shards/big", vec![], &striped()).await;
    assert_eq!(put.status, 200, "{}", put.body);
    member
}

async fn read_back(member: &Member) -> Vec<u8> {
    let read = call(&member.state, "GET", "/shards/big", vec![], b"").await;
    assert_eq!(read.status, 200, "{}", read.body);
    read.bytes
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_erasure_coded_object_reads_back_whole_and_by_range_across_stripes() {
    let _why = IGNORED;
    let member = written().await;
    assert_eq!(read_back(&member).await, striped());
    // The second stripe starts at 1 MiB; this range crosses into it.
    let ranged = call_with(
        &member.state,
        "GET",
        "/shards/big",
        vec![],
        vec![("range", "bytes=1048000-1049999")],
        b"",
    )
    .await;
    assert_eq!(ranged.status, 206, "{}", ranged.body);
    assert_eq!(ranged.bytes, striped()[1_048_000..1_050_000].to_vec());
    assert_eq!(
        count(&member.planter, "heals").await,
        0,
        "nothing was missing"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_missing_data_shard_is_rebuilt_from_parity_and_listed_for_healing() {
    let _why = IGNORED;
    let member = written().await;
    std::fs::remove_file(shard_file(&member, 1)).expect("shard removed");
    assert_eq!(read_back(&member).await, striped());
    assert_eq!(count(&member.planter, "heals").await, 1, "heal on read");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_damaged_data_shard_is_never_served_and_is_listed_for_healing() {
    let _why = IGNORED;
    let member = written().await;
    let path = shard_file(&member, 0);
    let mut bytes = std::fs::read(&path).expect("shard read");
    // Inside the first block, past the 32-byte header.
    let flipped = bytes.get_mut(132).expect("long enough");
    *flipped ^= 0xff;
    std::fs::write(&path, bytes).expect("shard damaged");
    assert_eq!(read_back(&member).await, striped());
    assert_eq!(count(&member.planter, "heals").await, 1, "heal on read");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn up_to_the_parity_shards_may_be_lost_and_one_more_refuses_the_read() {
    let _why = IGNORED;
    let member = written().await;
    for index in [0, 3] {
        std::fs::remove_file(shard_file(&member, index)).expect("shard removed");
    }
    assert_eq!(read_back(&member).await, striped(), "two lost");
    std::fs::remove_file(shard_file(&member, 5)).expect("shard removed");
    let read = call(&member.state, "GET", "/shards/big", vec![], b"").await;
    assert_eq!(
        (read.status, read.code.as_deref()),
        (503, Some("ServiceUnavailable")),
        "{}",
        read.body
    );
}
