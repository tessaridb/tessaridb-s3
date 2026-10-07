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
    drop(abandoned);
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
