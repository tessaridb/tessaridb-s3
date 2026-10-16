//! Sealed objects on the RS(4, 2) cluster of the `erasure` module (G017 W3c): every shard of a sealed object is a
//! sealed shard (format 4) holding no plaintext, the object reads back whole and by range, and a lost shard is
//! rebuilt as it was; a plain object beside it keeps plain shards (format 2) the scan can see into.

use std::path::{Path, PathBuf};

use tessari_s3_constants::DATA_BLOCK_SIZE;
use tessari_s3_storage::Error as StorageError;
use tessari_s3_storage::objects::Content;
use tessari_s3_types::{BucketName, ObjectKey};

use crate::erasure::{Member, member};
use crate::sse::{SSE, body, sse_header, unseal};
use crate::{IGNORED, call, call_with};

/// The data id `bucket/key` is stored under.
async fn data_of(member: &Member, bucket: &str, key: &str) -> [u8; 16] {
    let object = member
        .storage
        .objects()
        .get(
            &BucketName::new(bucket).expect("bucket"),
            &ObjectKey::new(key).expect("key"),
        )
        .await
        .expect("read")
        .ok()
        .flatten()
        .expect("stored");
    match object.content {
        Content::Data(id) => id,
        Content::Parts(_) => panic!("one data id"),
    }
}

/// Every shard file of data `id`, on whichever node holds it.
fn shards_of(member: &Member, id: [u8; 16]) -> Vec<PathBuf> {
    let name: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut found = Vec::new();
    for dir in std::iter::once(&member.dir).chain(member.peers.iter().map(|peer| &peer.dir)) {
        collect(dir, &format!("{name}.s"), &mut found);
    }
    found
}

fn collect(dir: &Path, prefix: &str, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_dir() {
            collect(&path, prefix, found);
        } else if path
            .file_name()
            .and_then(|file| file.to_str())
            .is_some_and(|file| file.starts_with(prefix) && !file.ends_with(".tmp"))
        {
            found.push(path);
        }
    }
}

/// The format a data file's header declares.
fn format_of(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[4], bytes[5]])
}

fn size_of(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).expect("small")
}

fn holds(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_sealed_object_is_sealed_in_every_shard_and_reads_and_heals_as_it_was() {
    let _why = IGNORED;
    let member = member().await;
    unseal(&member.state).await;
    let state = &member.state;
    assert_eq!(call(state, "PUT", "/shards", vec![], b"").await.status, 200);
    let bytes = body();
    let needle = &bytes[..32];
    let put = call_with(state, "PUT", "/shards/sealed", vec![], vec![SSE], &bytes).await;
    assert_eq!(put.status, 200, "{}", put.body);
    let plain = call(state, "PUT", "/shards/plain", vec![], &bytes).await;
    assert_eq!(plain.status, 200, "{}", plain.body);

    let id = data_of(&member, "shards", "sealed").await;
    // Read plain, at the object's size or at its stored size, the sealed shards are refused rather than served.
    let plain_block = u64::from(DATA_BLOCK_SIZE - 16);
    let stored = size_of(&bytes) + 16 * size_of(&bytes).div_ceil(plain_block);
    for size in [size_of(&bytes), stored] {
        assert!(
            matches!(
                member.storage.objects().open(id, size).await,
                Err(StorageError::Corrupt {
                    reason: "the shard set does not hold what the object names",
                    ..
                })
            ),
            "a plain read of {size} bytes"
        );
    }
    let sealed = shards_of(&member, id);
    assert_eq!(sealed.len(), 6, "k + m shards");
    for path in &sealed {
        let shard = std::fs::read(path).expect("shard read");
        assert_eq!(format_of(&shard), 4, "{} is a sealed shard", path.display());
        assert!(!holds(&shard, needle), "{} holds plaintext", path.display());
    }
    // Control: the plain object's shards are plain, and its data shards hold the canary the scan looks for.
    let open = shards_of(&member, data_of(&member, "shards", "plain").await);
    assert_eq!(open.len(), 6);
    let mut seen = false;
    for path in &open {
        let shard = std::fs::read(path).expect("shard read");
        assert_eq!(format_of(&shard), 2, "{} is a plain shard", path.display());
        seen |= holds(&shard, needle);
    }
    assert!(seen, "control: the scan finds plaintext in a plain shard");

    let got = call(state, "GET", "/shards/sealed", vec![], b"").await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(got.bytes, bytes);
    assert_eq!(sse_header(&got), Some(&b"AES256"[..]));
    let ranged = call_with(
        state,
        "GET",
        "/shards/sealed",
        vec![],
        vec![("range", "bytes=1048000-1049999")],
        b"",
    )
    .await;
    assert_eq!(ranged.status, 206, "{}", ranged.body);
    assert_eq!(ranged.bytes, bytes[1_048_000..1_050_000].to_vec());

    // A lost sealed data shard (index 1, which every whole read needs) is rebuilt byte for byte, still sealed.
    let lost = sealed
        .iter()
        .find(|path| path.to_string_lossy().ends_with(".s1"))
        .expect("shard 1");
    let original = std::fs::read(lost).expect("shard read");
    std::fs::remove_file(lost).expect("shard removed");
    let degraded = call(state, "GET", "/shards/sealed", vec![], b"").await;
    assert_eq!(
        degraded.bytes, bytes,
        "read past the lost shard, and listed"
    );
    let healed = member.storage.objects().heal(100).await.expect("heal pass");
    assert_eq!((healed.examined, healed.healed, healed.at_risk), (1, 1, 0));
    assert_eq!(std::fs::read(lost).expect("shard back"), original);
}
