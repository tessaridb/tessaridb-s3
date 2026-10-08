use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use tessari_s3_constants::DATA_BLOCK_SIZE;

use super::ShardService;
use crate::Error;
use crate::data::DataFiles;

const BLOCK: u32 = 64;
const ID: [u8; 16] = [7; 16];

/// A fresh directory under the system temp dir, removed by the test that made it.
fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("tessari-s3-data-{}", uuid::Uuid::new_v4().simple()))
}

fn service(root: &Path) -> (ShardService, DataFiles) {
    let files = DataFiles::new(root.to_path_buf(), DATA_BLOCK_SIZE);
    (ShardService::new(Some(files.clone())), files)
}

/// 2.5 blocks of distinct bytes.
fn body() -> Vec<u8> {
    (0..160_u32)
        .map(|i| u8::try_from(i % 251).expect("small"))
        .collect()
}

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).expect("small")
}

/// Neither the shard nor its temporary file exists.
fn nothing_left(files: &DataFiles, index: u16) {
    let path = files.shard_path(ID, index);
    let mut temp = path.clone().into_os_string();
    temp.push(".tmp");
    assert!(!path.exists(), "no shard under its final name");
    assert!(!PathBuf::from(temp).exists(), "no temporary file");
}

#[tokio::test]
async fn a_shard_is_written_read_back_and_removed() {
    let root = scratch();
    let (shards, files) = service(&root);
    let bytes = body();
    let mut write = shards
        .write(ID, 3, BLOCK, len(&bytes))
        .await
        .expect("write");
    for chunk in bytes.chunks(50) {
        write.append(chunk).await.expect("append");
    }
    write.finish().await.expect("finish");
    let reader = shards.open(ID, 3, len(&bytes)).await.expect("open");
    let mut read = Vec::new();
    for block in 0..3 {
        read.extend(reader.read_block(block).await.expect("block"));
    }
    assert_eq!(read, bytes);
    shards.remove(ID, 3).await.expect("remove");
    assert!(matches!(
        shards.open(ID, 3, len(&bytes)).await,
        Err(Error::DataIo {
            kind: ErrorKind::NotFound,
            ..
        })
    ));
    nothing_left(&files, 3);
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn a_block_size_or_index_out_of_range_is_refused_before_any_file() {
    let root = scratch();
    let (shards, files) = service(&root);
    for (index, block) in [(0, 0), (0, DATA_BLOCK_SIZE + 1), (16, BLOCK)] {
        assert!(
            matches!(
                shards.write(ID, index, block, 1).await,
                Err(Error::InvalidShard { .. })
            ),
            "index {index}, block {block}"
        );
        nothing_left(&files, index);
    }
    assert!(matches!(
        shards.open(ID, 16, 1).await,
        Err(Error::InvalidShard { .. })
    ));
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn a_body_longer_than_declared_is_refused_and_leaves_nothing() {
    let root = scratch();
    let (shards, files) = service(&root);
    let bytes = body();
    let declared = len(&bytes) - 1;
    let mut write = shards.write(ID, 1, BLOCK, declared).await.expect("write");
    assert_eq!(
        write.append(&bytes).await,
        Err(Error::ShardLength {
            declared,
            received: len(&bytes)
        })
    );
    write.abandon().await.expect("abandon");
    nothing_left(&files, 1);
    let mut again = shards.write(ID, 1, BLOCK, declared).await.expect("again");
    again.append(&bytes[..159]).await.expect("append");
    again.finish().await.expect("a retry writes the shard");
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn a_body_shorter_than_declared_is_refused_and_leaves_nothing() {
    let root = scratch();
    let (shards, files) = service(&root);
    let bytes = body();
    let mut write = shards
        .write(ID, 2, BLOCK, len(&bytes) + 1)
        .await
        .expect("write");
    write.append(&bytes).await.expect("append");
    assert_eq!(
        write.finish().await,
        Err(Error::ShardLength {
            declared: len(&bytes) + 1,
            received: len(&bytes)
        })
    );
    nothing_left(&files, 2);
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn without_a_data_directory_nothing_is_written() {
    let shards = ShardService::new(None);
    assert!(matches!(
        shards.write(ID, 0, BLOCK, 1).await,
        Err(Error::NoDataDirectory)
    ));
}
