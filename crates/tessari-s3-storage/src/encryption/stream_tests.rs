use std::os::unix::fs::FileExt;
use std::path::PathBuf;

use tessari_s3_core::objects::checksum::Hashes;

use super::{Sealer, plain_block, sealed_size};
use crate::Error;
use crate::data::{DataFiles, block_hash, block_span};
use crate::encryption::{BLOCK_TAG_LEN, DataKey, WrappedKey};
use crate::objects::{ObjectReader, Upload, Uploaded};

/// Stored block size; a sealed block carries [`PLAIN`] bytes of the object.
const BLOCK: u32 = 64;
const PLAIN: usize = 48;

fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!(
        "tessari-s3-sealed-{}",
        uuid::Uuid::new_v4().simple()
    ))
}

/// A wrapped key the upload carries along; nothing here unwraps it.
fn wrapped() -> WrappedKey {
    WrappedKey {
        algorithm: "xchacha20poly1305".to_owned(),
        kek_id: "server_1".to_owned(),
        nonce: vec![0; 24],
        ciphertext: vec![0; 48],
    }
}

fn key(byte: u8) -> DataKey {
    DataKey([byte; 32])
}

fn id(n: u8) -> [u8; 16] {
    [n; 16]
}

/// `len` distinct bytes that never look like a header or a hash.
fn body(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 241).expect("small"))
        .collect()
}

fn size_of(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).expect("small")
}

/// `bytes` written sealed under `key` as data `data_id`, appended `piece` bytes at a time.
async fn seal(
    files: &DataFiles,
    data_id: [u8; 16],
    key: DataKey,
    bytes: &[u8],
    piece: usize,
) -> Uploaded {
    let writer = files.create_sealed(data_id).await.expect("create");
    let sealer = Sealer::new(key, data_id, size_of(bytes), files.block_size()).expect("sealer");
    let mut upload = Upload::new(data_id, writer).sealing(sealer, wrapped());
    for chunk in bytes.chunks(piece.max(1)) {
        upload.append(chunk).await.expect("append");
    }
    upload.finish().await.expect("finish")
}

async fn open(files: &DataFiles, data_id: [u8; 16], key: DataKey, size: u64) -> ObjectReader {
    let stored = sealed_size(size, files.block_size()).expect("stored size");
    let inner = files.open_sealed(data_id, stored).await.expect("open");
    ObjectReader::sealed(ObjectReader::File(inner), key, data_id, size).expect("sealed reader")
}

async fn read_all(reader: &ObjectReader) -> Result<Vec<u8>, Error> {
    let block = u64::from(reader.block_size());
    let mut read = Vec::new();
    for index in 0..reader.size().div_ceil(block) {
        read.extend(reader.read_block(index).await?);
    }
    Ok(read)
}

/// Rewrites stored block `index` of `data_id` (its hash too, so the drive's own check passes and only the seal can
/// object).
fn rewrite(files: &DataFiles, data_id: [u8; 16], stored: u64, index: u64, block: &[u8]) {
    let (offset, length) = block_span(index, stored, BLOCK).expect("span");
    assert_eq!(size_of(block), length, "a rewrite keeps the block's length");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(files.path(data_id))
        .expect("open for rewrite");
    file.write_all_at(block, offset).expect("block");
    let hash_at = offset.checked_add(length).expect("offset");
    file.write_all_at(&block_hash(index, block), hash_at)
        .expect("hash");
}

fn stored_block(files: &DataFiles, data_id: [u8; 16], stored: u64, index: u64) -> Vec<u8> {
    let (offset, length) = block_span(index, stored, BLOCK).expect("span");
    let mut block = vec![0; usize::try_from(length).expect("small")];
    std::fs::File::open(files.path(data_id))
        .expect("open")
        .read_exact_at(&mut block, offset)
        .expect("read");
    block
}

#[test]
fn a_sealed_block_carries_the_stored_block_less_its_tag() {
    assert_eq!(plain_block(BLOCK), Some(BLOCK - 16));
    assert_eq!(
        usize::try_from(BLOCK).expect("small") - BLOCK_TAG_LEN,
        PLAIN
    );
    assert_eq!(
        plain_block(16),
        None,
        "a block that holds only a tag carries nothing"
    );
    assert_eq!(sealed_size(0, BLOCK), Some(0));
    assert_eq!(sealed_size(48, BLOCK), Some(64));
    assert_eq!(sealed_size(49, BLOCK), Some(49 + 32));
    assert_eq!(sealed_size(149, BLOCK), Some(149 + 64));
    assert_eq!(sealed_size(u64::MAX, BLOCK), None);
}

#[tokio::test]
async fn sealed_bytes_read_back_whole_at_every_boundary() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    for (n, len) in [0, PLAIN - 1, PLAIN, PLAIN + 1, 3 * PLAIN + 5]
        .into_iter()
        .enumerate()
    {
        let data_id = id(u8::try_from(n).expect("small").saturating_add(1));
        let bytes = body(len);
        let uploaded = seal(&files, data_id, key(9), &bytes, 7).await;
        assert_eq!(
            uploaded.size,
            size_of(&bytes),
            "the object's size, not the stored size"
        );
        let mut plain = Hashes::new();
        plain.update(&bytes);
        assert_eq!(
            uploaded.digests,
            plain.finish(),
            "digests of the plaintext at {len}"
        );
        let stored = sealed_size(size_of(&bytes), BLOCK).expect("stored");
        let header = files.path(data_id);
        assert_eq!(
            std::fs::metadata(&header).expect("metadata").len(),
            32 + stored + 32 * stored.div_ceil(u64::from(BLOCK)),
            "the file lays out the sealed size at {len}"
        );
        let reader = open(&files, data_id, key(9), size_of(&bytes)).await;
        assert_eq!(reader.block_size(), 48);
        assert_eq!(reader.size(), size_of(&bytes));
        assert_eq!(
            read_all(&reader).await.expect("read"),
            bytes,
            "round trip at {len}"
        );
    }
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn one_block_reads_alone() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes = body(3 * PLAIN + 5);
    seal(&files, id(1), key(9), &bytes, 100).await;
    let reader = open(&files, id(1), key(9), size_of(&bytes)).await;
    assert_eq!(
        reader.read_block(2).await.expect("block 2"),
        bytes[2 * PLAIN..3 * PLAIN]
    );
    assert_eq!(
        reader.read_block(3).await.expect("last"),
        bytes[3 * PLAIN..]
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn the_drive_holds_no_plaintext() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes: Vec<u8> = b"the canary that must never reach a drive "
        .iter()
        .copied()
        .cycle()
        .take(3 * PLAIN)
        .collect();
    let needle = &bytes[..20];
    seal(&files, id(1), key(9), &bytes, 30).await;
    let sealed = std::fs::read(files.path(id(1))).expect("read");
    assert!(
        !sealed.windows(needle.len()).any(|w| w == needle),
        "sealed file holds the plaintext"
    );
    // Control: the same bytes written plain are found, so the scan can see them.
    let mut writer = files.create(id(2)).await.expect("create");
    writer.append(&bytes).await.expect("append");
    writer.finish().await.expect("finish");
    let plain = std::fs::read(files.path(id(2))).expect("read");
    assert!(
        plain.windows(needle.len()).any(|w| w == needle),
        "the scan finds plain bytes"
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[path = "stream_refusal_tests.rs"]
mod refusals;
