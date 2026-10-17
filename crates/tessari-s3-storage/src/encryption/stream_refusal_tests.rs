//! What must NOT open: a changed byte, another key, a moved or cut block, bytes moved to another data id, a body of
//! the wrong length, and a sealed file read as plain.

use std::os::unix::fs::FileExt;

use super::super::{Sealer, sealed_size};
use super::{
    BLOCK, PLAIN, body, id, key, open, rewrite, scratch, seal, size_of, stored_block, wrapped,
};
use crate::Error;
use crate::data::{DataFiles, Piece, encode_header};
use crate::objects::Upload;

#[tokio::test]
async fn a_changed_byte_another_key_or_a_moved_block_does_not_open() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes = body(3 * PLAIN);
    let size = size_of(&bytes);
    let stored = sealed_size(size, BLOCK).expect("stored");
    seal(&files, id(1), key(9), &bytes, 48).await;

    let wrong = open(&files, id(1), key(8), size).await;
    assert!(
        matches!(wrong.read_block(0).await, Err(Error::Unsealable)),
        "another key"
    );

    let original = stored_block(&files, id(1), stored, 1);
    let mut flipped = original.clone();
    flipped[5] ^= 1;
    rewrite(&files, id(1), stored, 1, &flipped);
    let reader = open(&files, id(1), key(9), size).await;
    assert!(
        matches!(reader.read_block(1).await, Err(Error::Unsealable)),
        "a changed byte"
    );
    assert_eq!(
        reader.read_block(0).await.expect("block 0"),
        bytes[..PLAIN],
        "the others still open"
    );
    rewrite(&files, id(1), stored, 1, &original);
    assert_eq!(
        reader.read_block(1).await.expect("restored"),
        bytes[PLAIN..2 * PLAIN]
    );

    let first = stored_block(&files, id(1), stored, 0);
    rewrite(&files, id(1), stored, 0, &original);
    rewrite(&files, id(1), stored, 1, &first);
    assert!(
        matches!(reader.read_block(0).await, Err(Error::Unsealable)),
        "block 1 at 0"
    );
    assert!(
        matches!(reader.read_block(1).await, Err(Error::Unsealable)),
        "block 0 at 1"
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn a_file_cut_at_a_block_boundary_does_not_open_as_a_shorter_object() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes = body(2 * PLAIN);
    seal(&files, id(1), key(9), &bytes, 48).await;
    // The first stored block alone, laid out as a one-block object: the drive's checks pass, the seal does not.
    let one = sealed_size(size_of(&bytes[..PLAIN]), BLOCK).expect("stored");
    let path = files.path(id(1));
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open")
        .set_len(32 + one + 32)
        .expect("truncate");
    let reader = open(&files, id(1), key(9), size_of(&bytes[..PLAIN])).await;
    assert!(
        matches!(reader.read_block(0).await, Err(Error::Unsealable)),
        "a non-last block read as the last"
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn sealed_bytes_moved_to_another_data_id_do_not_open() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes = body(PLAIN + 3);
    seal(&files, id(1), key(9), &bytes, 48).await;
    let moved = files.path(id(2));
    std::fs::create_dir_all(moved.parent().expect("dir")).expect("dir");
    std::fs::copy(files.path(id(1)), &moved).expect("copy");
    // Its header rewritten for the new id, so only the seal's associated data can tell.
    let header = encode_header(BLOCK, id(2), Piece::Whole, true);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&moved)
        .expect("open")
        .write_all_at(&header, 0)
        .expect("header");
    let reader = open(&files, id(2), key(9), size_of(&bytes)).await;
    assert!(matches!(reader.read_block(0).await, Err(Error::Unsealable)));
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn a_sealed_upload_holds_to_its_declared_length() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let long = {
        let writer = files.create_sealed(id(1)).await.expect("create");
        let mut upload = Upload::new(id(1), writer).sealing(
            Sealer::new(key(9), id(1), 10, BLOCK).expect("sealer"),
            wrapped(),
        );
        upload.append(&body(11)).await
    };
    assert!(
        matches!(
            long,
            Err(Error::SealedLength {
                declared: 10,
                received: 11
            })
        ),
        "{long:?}"
    );
    let writer = files.create_sealed(id(2)).await.expect("create");
    let mut upload = Upload::new(id(2), writer).sealing(
        Sealer::new(key(9), id(2), 100, BLOCK).expect("sealer"),
        wrapped(),
    );
    upload.append(&body(60)).await.expect("append");
    let short = upload.finish().await;
    assert!(
        matches!(
            short,
            Err(Error::SealedLength {
                declared: 100,
                received: 60
            })
        ),
        "{short:?}"
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn a_plain_reader_refuses_a_sealed_file() {
    let root = scratch();
    let files = DataFiles::new(root.clone(), BLOCK);
    let bytes = body(PLAIN);
    seal(&files, id(1), key(9), &bytes, 48).await;
    let stored = sealed_size(size_of(&bytes), BLOCK).expect("stored");
    assert!(matches!(
        files.open(id(1), stored).await,
        Err(Error::Corrupt {
            reason: "header",
            ..
        })
    ));
    std::fs::remove_dir_all(root).expect("cleanup");
}
