use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tessari_s3_constants::MULTIPART_MIN_PART_SIZE;

use super::super::checksum::{ChecksumAlgorithm, Hashes};
use super::super::crc_combine::combine;
use super::{ListedPart, ObjectChecksum, PartRecord, Refusal, assemble};

const CRCS: [ChecksumAlgorithm; 3] = [
    ChecksumAlgorithm::Crc32,
    ChecksumAlgorithm::Crc32c,
    ChecksumAlgorithm::Crc64Nvme,
];

/// A CRC as a number, through the same streaming hasher the server stores part CRCs from.
fn crc(bytes: &[u8], algorithm: ChecksumAlgorithm) -> u64 {
    let mut hashes = Hashes::new();
    hashes.update(bytes);
    let raw = STANDARD
        .decode(hashes.finish().value(algorithm))
        .expect("base64");
    raw.iter()
        .fold(0, |value, byte| (value << 8) | u64::from(*byte))
}

#[test]
fn a_combined_crc_is_the_crc_of_the_joined_bytes_for_every_split() {
    let bytes: Vec<u8> = (0_u32..3000)
        .map(|i| u8::try_from(i.wrapping_mul(31).wrapping_add(7) % 251).expect("small"))
        .collect();
    for algorithm in CRCS {
        for split in [0, 1, 7, 8, 9, 255, 1024, 2999, 3000] {
            let (first, second) = bytes.split_at(split);
            let second_len = u64::try_from(second.len()).expect("small");
            assert_eq!(
                combine(
                    algorithm,
                    crc(first, algorithm),
                    crc(second, algorithm),
                    second_len
                ),
                Some(crc(&bytes, algorithm)),
                "{} split at {split}",
                algorithm.name()
            );
        }
    }
    assert_eq!(
        combine(ChecksumAlgorithm::Sha256, 1, 2, 3),
        None,
        "not a CRC"
    );
}

fn record(bytes: &[u8], size: u64) -> PartRecord {
    let mut hashes = Hashes::new();
    hashes.update(bytes);
    let digests = hashes.finish();
    PartRecord {
        size,
        etag: format!("\"{}\"", digests.md5_hex),
        checksums: ChecksumAlgorithm::ALL
            .into_iter()
            .map(|a| (a.name().to_owned(), digests.value(a).to_owned()))
            .collect(),
    }
}

/// Part 1 (`aaaaaaa`, recorded at the 5 MiB minimum so it may precede another) and part 2 (`bbbbb`).
fn stored() -> BTreeMap<u16, PartRecord> {
    [
        (1, record(&[b'a'; 7], MULTIPART_MIN_PART_SIZE)),
        (2, record(&[b'b'; 5], 5)),
    ]
    .into()
}

fn listed(parts: &[(u16, &str)]) -> Vec<ListedPart> {
    parts
        .iter()
        .map(|(number, etag)| ListedPart {
            number: *number,
            etag: (*etag).to_owned(),
            checksums: Vec::new(),
        })
        .collect()
}

const A: &str = "\"5d793fc5b00a2348c3fb9ab59e5ca98a\"";
const B: &str = "5d793fc5b00a2348c3fb9ab59e5ca98a";
const SECOND: &str = "a21075a36eeddd084e17611a238c7101";

/// Known answers computed outside this code base (Python `hashlib` and `zlib`).
#[test]
fn the_object_carries_the_published_etag_and_checksums() {
    let crc32 = assemble(
        &listed(&[(1, A), (2, SECOND)]),
        &stored(),
        ObjectChecksum::FullObject(ChecksumAlgorithm::Crc32),
    )
    .expect("assembles");
    assert_eq!(crc32.etag, "\"78ff1c6c534a0c84b77b1723fc2bb89e-2\"");
    assert_eq!(crc32.size, MULTIPART_MIN_PART_SIZE + 5);
    assert_eq!(crc32.checksum, "E5RjaA==", "zlib.crc32 of the joined bytes");
    let sha256 = assemble(
        &listed(&[(1, B), (2, SECOND)]),
        &stored(),
        ObjectChecksum::Composite(ChecksumAlgorithm::Sha256),
    )
    .expect("assembles; an unquoted ETag matches");
    assert_eq!(
        sha256.checksum,
        "GoGrd64hXzHhY/CMcsiS7EorxStzbmytmiXL+eTRocg=-2"
    );
}

#[test]
fn a_list_the_parts_cannot_honour_is_refused_by_name() {
    let full = ObjectChecksum::FullObject(ChecksumAlgorithm::Crc64Nvme);
    let refused = |parts: &[(u16, &str)], stored: &BTreeMap<u16, PartRecord>, checksum| {
        assemble(&listed(parts), stored, checksum).expect_err("refused")
    };
    assert_eq!(
        refused(&[(2, SECOND), (1, A)], &stored(), full),
        Refusal::Order
    );
    assert_eq!(refused(&[(1, A), (1, A)], &stored(), full), Refusal::Order);
    assert_eq!(
        refused(&[(1, A), (3, SECOND)], &stored(), full),
        Refusal::Part(3)
    );
    assert_eq!(refused(&[(1, SECOND)], &stored(), full), Refusal::Part(1));
    assert_eq!(refused(&[], &stored(), full), Refusal::Count);
    let mut small_first = stored();
    small_first.insert(1, record(&[b'a'; 7], MULTIPART_MIN_PART_SIZE - 1));
    assert_eq!(
        refused(&[(1, A), (2, SECOND)], &small_first, full),
        Refusal::TooSmall(1)
    );
    assert!(
        assemble(&listed(&[(2, SECOND)]), &stored(), full).is_ok(),
        "the last part has no minimum"
    );
    let mut gap = stored();
    gap.insert(3, record(&[b'b'; 5], 5));
    gap.insert(2, record(&[b'a'; 7], MULTIPART_MIN_PART_SIZE));
    assert_eq!(
        refused(
            &[(1, A), (3, SECOND)],
            &gap,
            ObjectChecksum::Composite(ChecksumAlgorithm::Sha1)
        ),
        Refusal::Part(2),
        "a composite checksum needs parts 1 to N"
    );
    let mut with_checksum = listed(&[(1, A), (2, SECOND)]);
    with_checksum[1]
        .checksums
        .push((ChecksumAlgorithm::Crc32, "AAAAAA==".to_owned()));
    assert_eq!(
        assemble(&with_checksum, &stored(), full),
        Err(Refusal::Part(2)),
        "a listed checksum must be the stored one"
    );
}

#[test]
fn only_the_pairs_s3_allows_are_declared() {
    use ChecksumAlgorithm::{Crc32, Crc32c, Crc64Nvme, Sha1, Sha256};
    let declared = ObjectChecksum::declared;
    assert_eq!(
        declared(None, None),
        Some(ObjectChecksum::FullObject(Crc64Nvme))
    );
    assert_eq!(
        declared(Some(Crc64Nvme), None),
        Some(ObjectChecksum::FullObject(Crc64Nvme))
    );
    assert_eq!(
        declared(Some(Crc32), None),
        Some(ObjectChecksum::Composite(Crc32))
    );
    assert_eq!(
        declared(Some(Crc32c), Some("FULL_OBJECT")),
        Some(ObjectChecksum::FullObject(Crc32c))
    );
    assert_eq!(
        declared(Some(Sha256), Some("COMPOSITE")),
        Some(ObjectChecksum::Composite(Sha256))
    );
    assert_eq!(declared(Some(Crc64Nvme), Some("COMPOSITE")), None);
    assert_eq!(declared(Some(Sha1), Some("FULL_OBJECT")), None);
    assert_eq!(declared(None, Some("COMPOSITE")), None);
}
