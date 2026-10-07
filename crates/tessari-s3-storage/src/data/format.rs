//! The data file format, version 1: a 32-byte header naming the file's data id and block size, then every block of
//! the object followed by its BLAKE3 hash. The object's size lives in its metadata record; the layout of the rest
//! follows from the size and the block size, so a reader can check the file's length before trusting it.
//!
//! ```text
//! header  magic "TS3S" | format u16 LE | reserved u16 | block size u32 LE | data id (16) | check (4)
//! block i bytes (block size, the last one shorter) | BLAKE3(i as u64 LE ‖ bytes) (32)
//! ```
//!
//! The header's check is the first four bytes of BLAKE3 over the 28 bytes before it. A block's hash covers its
//! index, so a block written at another position fails as surely as a flipped bit.

/// The first four bytes of every data file.
pub(crate) const MAGIC: [u8; 4] = *b"TS3S";
/// The format this build writes; it reads this one only (the first).
pub(crate) const FORMAT: u16 = 1;
/// Header length in bytes.
pub(crate) const HEADER_LEN: usize = 32;
/// Length of the hash after each block.
pub(crate) const HASH_LEN: usize = 32;
/// [`HEADER_LEN`] as a file offset.
const HEADER_BYTES: u64 = 32;
/// [`HASH_LEN`] as a file offset.
const HASH_BYTES: u64 = 32;

/// Why a header was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderFault {
    /// Not a data file.
    Magic,
    /// A format this build does not read.
    Format(u16),
    /// The header's own check failed.
    Check,
    /// The file belongs to another data id.
    Id,
    /// A block size of zero.
    BlockSize,
}

/// The header for data `id` written in blocks of `block_size` bytes.
pub(crate) fn encode_header(block_size: u32, id: [u8; 16]) -> [u8; HEADER_LEN] {
    let mut header = [0; HEADER_LEN];
    header[0..4].copy_from_slice(&MAGIC);
    header[4..6].copy_from_slice(&FORMAT.to_le_bytes());
    header[8..12].copy_from_slice(&block_size.to_le_bytes());
    header[12..28].copy_from_slice(&id);
    let check = header_check(&header);
    header[28..32].copy_from_slice(&check);
    header
}

/// The first four bytes of BLAKE3 over everything in the header before the check.
fn header_check(header: &[u8; HEADER_LEN]) -> [u8; 4] {
    let digest = blake3::hash(&header[0..28]);
    let mut check = [0; 4];
    check.copy_from_slice(&digest.as_bytes()[0..4]);
    check
}

/// The block size a header declares, after checking that it is a version-1 header for data `id`.
pub(crate) fn decode_header(header: &[u8; HEADER_LEN], id: [u8; 16]) -> Result<u32, HeaderFault> {
    if header[0..4] != MAGIC {
        return Err(HeaderFault::Magic);
    }
    let format = u16::from_le_bytes([header[4], header[5]]);
    if format != FORMAT {
        return Err(HeaderFault::Format(format));
    }
    if header[28..32] != header_check(header) {
        return Err(HeaderFault::Check);
    }
    if header[12..28] != id {
        return Err(HeaderFault::Id);
    }
    let block_size = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if block_size == 0 {
        return Err(HeaderFault::BlockSize);
    }
    Ok(block_size)
}

/// The hash stored after block `index`.
pub(crate) fn block_hash(index: u64, block: &[u8]) -> [u8; HASH_LEN] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&index.to_le_bytes());
    hasher.update(block);
    *hasher.finalize().as_bytes()
}

/// Number of blocks an object of `size` bytes occupies; `None` for a zero block size.
pub(crate) fn block_count(size: u64, block_size: u32) -> Option<u64> {
    size.checked_div(u64::from(block_size))
        .zip(size.checked_rem(u64::from(block_size)))
        .and_then(|(whole, rest)| whole.checked_add(u64::from(rest != 0)))
}

/// Bytes a block occupies in the file: its data and its hash.
fn stride(block_size: u32) -> Option<u64> {
    u64::from(block_size).checked_add(HASH_BYTES)
}

/// Total file length for an object of `size` bytes; `None` when it overflows or the block size is zero.
pub(crate) fn file_len(size: u64, block_size: u32) -> Option<u64> {
    let hashes = block_count(size, block_size)?.checked_mul(HASH_BYTES)?;
    (HEADER_BYTES).checked_add(size)?.checked_add(hashes)
}

/// Where block `index` starts in the file and how many object bytes it holds; `None` past the last block.
pub(crate) fn block_span(index: u64, size: u64, block_size: u32) -> Option<(u64, u64)> {
    if index >= block_count(size, block_size)? {
        return None;
    }
    let before = index.checked_mul(u64::from(block_size))?;
    let length = size.checked_sub(before)?.min(u64::from(block_size));
    let offset = (HEADER_BYTES).checked_add(index.checked_mul(stride(block_size)?)?)?;
    Some((offset, length))
}

#[cfg(test)]
#[path = "format_tests.rs"]
mod tests;
