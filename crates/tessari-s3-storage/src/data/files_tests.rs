use std::path::PathBuf;

use tessari_s3_core::objects::checksum::Hashes;

use super::DataFiles;
use crate::Error;
use crate::data::format::{HEADER_LEN, file_len};

const BLOCK: u32 = 64;

/// A fresh directory under the system temp dir, removed by the test that made it.
fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("tessari-s3-data-{}", uuid::Uuid::new_v4().simple()))
}

fn id(n: u8) -> [u8; 16] {
    [n; 16]
}

/// 2.5 blocks of distinct bytes.
fn body() -> Vec<u8> {
    (0..160_u32)
        .map(|i| u8::try_from(i % 251).expect("small"))
        .collect()
}

async fn written(files: &DataFiles, data_id: [u8; 16], bytes: &[u8], piece: usize) {
    let mut writer = files.create(data_id).await.expect("create");
    for chunk in bytes.chunks(piece) {
        writer.append(chunk).await.expect("append");
    }
    let finished = writer.finish().await.expect("finish");
    let mut expected = Hashes::new();
    expected.update(bytes);
    assert_eq!(
        finished.digests,
        expected.finish(),
        "digests of the whole body"
    );
    assert_eq!(finished.size, u64::try_from(bytes.len()).expect("small"));
}

#[tokio::test]
async fn a_file_reads_back_block_by_block_under_its_final_name_only() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes = body();
    written(&files, id(1), &bytes, 7).await;
    let path = files.path(id(1));
    assert!(path.exists(), "final name present");
    assert!(!path.with_extension("tmp").exists(), "no temp file left");
    let size = u64::try_from(bytes.len()).expect("small");
    assert_eq!(
        std::fs::metadata(&path).expect("metadata").len(),
        file_len(size, BLOCK).expect("len")
    );
    let reader = files.open(id(1), size).await.expect("open");
    let mut read = Vec::new();
    for index in 0..3 {
        read.extend(reader.read_block(index).await.expect("block"));
    }
    assert_eq!(read, bytes);
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[tokio::test]
async fn a_flipped_byte_in_a_block_is_refused_as_corrupt() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes = body();
    written(&files, id(2), &bytes, 64).await;
    let path = files.path(id(2));
    let mut raw = std::fs::read(&path).expect("read");
    // A byte in the middle of block 1.
    raw[HEADER_LEN + 64 + 32 + 10] ^= 0x40;
    std::fs::write(&path, &raw).expect("tamper");
    let reader = files
        .open(id(2), u64::try_from(bytes.len()).expect("small"))
        .await
        .expect("header still fine");
    assert!(reader.read_block(0).await.is_ok(), "block 0 untouched");
    assert!(
        matches!(
            reader.read_block(1).await,
            Err(Error::Corrupt {
                reason: "block hash",
                ..
            })
        ),
        "the damaged block is refused"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[tokio::test]
async fn a_damaged_header_or_a_wrong_length_is_refused_on_open() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes = body();
    let size = u64::try_from(bytes.len()).expect("small");
    written(&files, id(3), &bytes, 100).await;
    assert!(
        matches!(
            files.open(id(3), size + 1).await,
            Err(Error::Corrupt {
                reason: "length",
                ..
            })
        ),
        "a size the file does not hold"
    );
    let path = files.path(id(3));
    let mut raw = std::fs::read(&path).expect("read");
    raw[0] = b'X';
    std::fs::write(&path, &raw).expect("tamper");
    assert!(
        matches!(
            files.open(id(3), size).await,
            Err(Error::Corrupt {
                reason: "header",
                ..
            })
        ),
        "a header that is not ours"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[tokio::test]
async fn remove_takes_the_file_and_an_abandoned_temp_away() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    written(&files, id(4), &body(), 64).await;
    let mut abandoned = files.create(id(5)).await.expect("create");
    abandoned.append(&body()).await.expect("append");
    // A crash: no destructor runs, so the temp stays.
    std::mem::forget(abandoned);
    assert!(
        files.path(id(5)).with_extension("tmp").exists(),
        "temp of an abandoned write"
    );
    files.remove(id(4)).await.expect("remove");
    files.remove(id(5)).await.expect("remove temp");
    files
        .remove(id(6))
        .await
        .expect("removing nothing succeeds");
    assert!(!files.path(id(4)).exists());
    assert!(!files.path(id(5)).with_extension("tmp").exists());
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn the_path_is_sharded_by_the_id_and_names_nothing_a_client_chose() {
    let files = DataFiles::new(PathBuf::from("/d"), BLOCK);
    let data_id = [0xab, 0xcd, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01];
    assert_eq!(
        files.path(data_id),
        PathBuf::from("/d/s3data/ab/cd/abcd0000000000000000000000000001")
    );
}

#[tokio::test]
async fn a_shard_reads_back_only_as_that_shard_of_that_data() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes = body();
    let size = u64::try_from(bytes.len()).expect("small");
    let mut writer = files.create_shard(id(5), 2, 48).await.expect("create");
    for chunk in bytes.chunks(13) {
        writer.append(chunk).await.expect("append");
    }
    assert_eq!(writer.finish().await.expect("finish").size, size);
    let reader = files.open_shard(id(5), 2, size).await.expect("open");
    assert_eq!(reader.block_size(), 48);
    let mut read = Vec::new();
    for index in 0..4 {
        read.extend(reader.read_block(index).await.expect("block"));
    }
    assert_eq!(read, bytes);
    assert!(
        matches!(files.open(id(5), size).await, Err(Error::DataIo { .. })),
        "a shard is not the whole object's file"
    );
    std::fs::rename(files.shard_path(id(5), 2), files.shard_path(id(5), 1)).expect("renamed");
    assert!(
        matches!(
            files.open_shard(id(5), 1, size).await,
            Err(Error::Corrupt {
                reason: "header",
                ..
            })
        ),
        "the header names shard 2"
    );
    files.remove_shard(id(5), 1).await.expect("removed");
    assert!(!files.shard_path(id(5), 1).exists());
    files
        .remove_shard(id(5), 1)
        .await
        .expect("removing nothing is not an error");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn a_write_cut_short_by_a_crash_is_cleared_at_start_and_the_piece_can_be_written_again() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    written(&files, id(7), &body(), 64).await;
    let mut cut = files.create_shard(id(8), 1, 48).await.expect("create");
    cut.append(&body()).await.expect("append");
    // The process died here: no destructor runs, and nothing removes the temp while the data stays referenced.
    std::mem::forget(cut);
    assert!(
        matches!(
            files.create_shard(id(8), 1, 48).await,
            Err(Error::DataIo {
                op: "create",
                kind: std::io::ErrorKind::AlreadyExists
            })
        ),
        "the leftover temp refuses a new writer of the same shard"
    );
    assert_eq!(files.recover().await.expect("recover"), 1, "one temp left");
    assert!(
        files.path(id(7)).exists(),
        "a committed file is not touched"
    );
    let mut again = files
        .create_shard(id(8), 1, 48)
        .await
        .expect("written again");
    again.append(&body()).await.expect("append");
    again.finish().await.expect("finish");
    assert_eq!(
        files.recover().await.expect("recover"),
        0,
        "nothing left after a clean run"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[tokio::test]
async fn a_write_dropped_unfinished_takes_its_temp_away_so_the_piece_can_be_written_again() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let mut cancelled = files.create_shard(id(9), 0, 48).await.expect("create");
    cancelled.append(&body()).await.expect("append");
    // A request whose client went away, or a heal that stopped: the future holding the writer is dropped.
    drop(cancelled);
    let mut again = files
        .create_shard(id(9), 0, 48)
        .await
        .expect("nothing is left in the way of the next writer");
    again.append(&body()).await.expect("append");
    again.finish().await.expect("finish");
    assert!(files.shard_path(id(9), 0).exists());
    std::fs::remove_dir_all(&root).expect("cleanup");
}
