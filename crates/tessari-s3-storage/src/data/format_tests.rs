use super::{
    FORMAT, HASH_BYTES, HASH_LEN, HEADER_BYTES, HEADER_LEN, HeaderFault, MAGIC, Piece, block_count,
    block_hash, block_span, decode_header, encode_header, file_len,
};

const ID: [u8; 16] = [7; 16];
const MIB: u32 = 1 << 20;

#[test]
fn a_header_round_trips_and_carries_magic_and_format() {
    let header = encode_header(MIB, ID, Piece::Whole);
    assert_eq!(header[0..4], MAGIC);
    assert_eq!(u16::from_le_bytes([header[4], header[5]]), FORMAT);
    assert_eq!(header[12..28], ID);
    assert_eq!(decode_header(&header, ID, Piece::Whole), Ok(MIB));
}

#[test]
fn a_header_is_refused_for_each_fault_by_name() {
    let good = encode_header(MIB, ID, Piece::Whole);
    let mut magic = good;
    magic[0] = b'X';
    assert_eq!(
        decode_header(&magic, ID, Piece::Whole),
        Err(HeaderFault::Magic)
    );
    let mut newer = good;
    newer[4] = 2;
    assert_eq!(
        decode_header(&newer, ID, Piece::Whole),
        Err(HeaderFault::Format(2))
    );
    let mut flipped = good;
    flipped[9] ^= 1;
    assert_eq!(
        decode_header(&flipped, ID, Piece::Whole),
        Err(HeaderFault::Check)
    );
    assert_eq!(
        decode_header(&good, [8; 16], Piece::Whole),
        Err(HeaderFault::Id)
    );
    assert_eq!(
        decode_header(&encode_header(0, ID, Piece::Whole), ID, Piece::Whole),
        Err(HeaderFault::BlockSize)
    );
}

#[test]
fn a_shard_header_names_its_index_and_is_never_read_as_a_whole_object() {
    let shard = encode_header(MIB, ID, Piece::Shard(3));
    assert_eq!(
        u16::from_le_bytes([shard[4], shard[5]]),
        2,
        "format 2 is a shard"
    );
    assert_eq!(decode_header(&shard, ID, Piece::Shard(3)), Ok(MIB));
    assert_eq!(
        decode_header(&shard, ID, Piece::Shard(4)),
        Err(HeaderFault::Piece),
        "a shard moved to another index's name"
    );
    assert_eq!(
        decode_header(&shard, ID, Piece::Whole),
        Err(HeaderFault::Format(2))
    );
    let whole = encode_header(MIB, ID, Piece::Whole);
    assert_eq!(
        decode_header(&whole, ID, Piece::Shard(0)),
        Err(HeaderFault::Format(1))
    );
}

#[test]
fn a_block_hash_is_blake3_and_binds_the_index() {
    let block = b"some object bytes";
    let mut expected = blake3::Hasher::new();
    expected.update(&3_u64.to_le_bytes());
    expected.update(block);
    assert_eq!(block_hash(3, block), *expected.finalize().as_bytes());
    assert_ne!(block_hash(3, block), block_hash(4, block));
    assert_ne!(block_hash(3, block), block_hash(3, b"some object bytez"));
}

#[test]
fn the_layout_arithmetic_follows_the_size_and_block_size() {
    let hash = u64::try_from(HASH_LEN).expect("small");
    let header = u64::try_from(HEADER_LEN).expect("small");
    let mib = u64::from(MIB);
    // 2.5 blocks: three blocks, the last one half full.
    let size = mib * 5 / 2;
    assert_eq!(block_count(size, MIB), Some(3));
    assert_eq!(block_count(mib, MIB), Some(1));
    assert_eq!(block_count(mib + 1, MIB), Some(2));
    assert_eq!(file_len(size, MIB), Some(header + size + 3 * hash));
    assert_eq!(block_span(0, size, MIB), Some((header, mib)));
    assert_eq!(block_span(1, size, MIB), Some((header + mib + hash, mib)));
    assert_eq!(
        block_span(2, size, MIB),
        Some((header + 2 * (mib + hash), mib / 2))
    );
    assert_eq!(block_span(3, size, MIB), None);
    assert_eq!(block_count(size, 0), None);
    assert_eq!(file_len(u64::MAX, MIB), None);
}

#[test]
fn the_offset_constants_agree_with_the_lengths() {
    assert_eq!(u64::try_from(HEADER_LEN).ok(), Some(HEADER_BYTES));
    assert_eq!(u64::try_from(HASH_LEN).ok(), Some(HASH_BYTES));
}
