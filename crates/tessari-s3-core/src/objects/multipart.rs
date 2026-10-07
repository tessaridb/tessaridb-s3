//! What CompleteMultipartUpload decides before anything is committed: whether the listed parts can form the object,
//! and the identities that object then carries — the multipart ETag, its size and its checksum.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tessari_s3_constants::{MULTIPART_MAX_PARTS, MULTIPART_MIN_PART_SIZE};

use super::checksum::{ChecksumAlgorithm, Hashes};
use super::crc_combine::combine;

/// The checksum a multipart object carries: the algorithm and whether it covers the whole object or the parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectChecksum {
    /// A CRC of the whole object, combined from the parts' CRCs.
    FullObject(ChecksumAlgorithm),
    /// The algorithm over the concatenated part checksums, with `-N`.
    Composite(ChecksumAlgorithm),
}

impl ObjectChecksum {
    /// The checksum an upload declaring `algorithm` and `kind` (`COMPOSITE` / `FULL_OBJECT`) produces; `None` for a
    /// pair S3 does not allow. Nothing declared is a full-object CRC64NVME, as S3 stores for every new object.
    #[must_use]
    pub fn declared(algorithm: Option<ChecksumAlgorithm>, kind: Option<&str>) -> Option<Self> {
        use ChecksumAlgorithm::{Crc32, Crc32c, Crc64Nvme, Sha1, Sha256};
        match (algorithm, kind) {
            (None, None) | (Some(Crc64Nvme), None | Some("FULL_OBJECT")) => {
                Some(Self::FullObject(Crc64Nvme))
            }
            (Some(crc @ (Crc32 | Crc32c)), Some("FULL_OBJECT")) => Some(Self::FullObject(crc)),
            (Some(algorithm @ (Crc32 | Crc32c | Sha1 | Sha256)), None | Some("COMPOSITE")) => {
                Some(Self::Composite(algorithm))
            }
            _ => None,
        }
    }

    /// The algorithm.
    #[must_use]
    pub const fn algorithm(self) -> ChecksumAlgorithm {
        match self {
            Self::FullObject(algorithm) | Self::Composite(algorithm) => algorithm,
        }
    }
}

/// One part as the Complete request lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedPart {
    /// The part number.
    pub number: u16,
    /// The ETag the client was given, quoted or not.
    pub etag: String,
    /// Checksums the client listed for it, base64.
    pub checksums: Vec<(ChecksumAlgorithm, String)>,
}

/// One stored part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartRecord {
    /// Size in bytes.
    pub size: u64,
    /// The quoted ETag: the hex MD5 of its bytes.
    pub etag: String,
    /// Its checksums by algorithm name, base64 — every supported algorithm, computed as its bytes arrived.
    pub checksums: BTreeMap<String, String>,
}

/// Why the listed parts cannot form the object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Not in strictly ascending part number: InvalidPartOrder.
    Order,
    /// This part is not stored, or its ETag or a listed checksum differs, or a composite checksum lacks it:
    /// InvalidPart.
    Part(u16),
    /// This part, not the last, is under 5 MiB: EntityTooSmall.
    TooSmall(u16),
    /// More parts than one upload may hold, or none.
    Count,
}

/// The object the listed parts form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    /// The quoted multipart ETag: the hex MD5 of the parts' binary MD5s, `-`, the count.
    pub etag: String,
    /// The object's size.
    pub size: u64,
    /// The object's checksum value, base64 (`-N` for a composite one).
    pub checksum: String,
}

fn unquoted(etag: &str) -> &str {
    etag.trim().trim_matches('"')
}

fn md5_of(etag: &str) -> Option<[u8; 16]> {
    let hex = unquoted(etag).as_bytes();
    let mut md5 = [0_u8; 16];
    if hex.len() != 32 {
        return None;
    }
    for (byte, pair) in md5.iter_mut().zip(hex.as_chunks::<2>().0) {
        let text = std::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(text, 16).ok()?;
    }
    Some(md5)
}

/// The stored raw digest of `algorithm` for a part.
fn raw(part: &PartRecord, algorithm: ChecksumAlgorithm) -> Option<Vec<u8>> {
    STANDARD.decode(part.checksums.get(algorithm.name())?).ok()
}

/// A CRC's raw big-endian bytes as a number.
fn crc_value(bytes: &[u8]) -> Option<u64> {
    match bytes.len() {
        4 => Some(u64::from(u32::from_be_bytes(bytes.try_into().ok()?))),
        8 => Some(u64::from_be_bytes(bytes.try_into().ok()?)),
        _ => None,
    }
}

/// Checks `listed` against the `stored` parts and computes the object they form under `checksum`.
///
/// # Errors
/// The first [`Refusal`], in the order S3 checks: the count, the order, each part, the sizes.
pub fn assemble(
    listed: &[ListedPart],
    stored: &BTreeMap<u16, PartRecord>,
    checksum: ObjectChecksum,
) -> Result<Assembled, Refusal> {
    if listed.is_empty() || listed.len() > usize::from(MULTIPART_MAX_PARTS) {
        return Err(Refusal::Count);
    }
    if listed
        .windows(2)
        .any(|pair| pair[0].number >= pair[1].number)
    {
        return Err(Refusal::Order);
    }
    let mut parts = Vec::with_capacity(listed.len());
    for entry in listed {
        let part = stored
            .get(&entry.number)
            .ok_or(Refusal::Part(entry.number))?;
        let checksums_agree = entry
            .checksums
            .iter()
            .all(|(algorithm, value)| part.checksums.get(algorithm.name()) == Some(value));
        if unquoted(&entry.etag) != unquoted(&part.etag) || !checksums_agree {
            return Err(Refusal::Part(entry.number));
        }
        parts.push((entry.number, part));
    }
    let (init, _last) = parts.split_at(parts.len().saturating_sub(1));
    if let Some((number, _)) = init
        .iter()
        .find(|(_, part)| part.size < MULTIPART_MIN_PART_SIZE)
    {
        return Err(Refusal::TooSmall(*number));
    }
    let mut md5s = Vec::with_capacity(parts.len().saturating_mul(16));
    let mut size: u64 = 0;
    for (number, part) in &parts {
        md5s.extend_from_slice(&md5_of(&part.etag).ok_or(Refusal::Part(*number))?);
        size = size.checked_add(part.size).ok_or(Refusal::Count)?;
    }
    let mut hashes = Hashes::new();
    hashes.update(&md5s);
    let etag = format!("\"{}-{}\"", hashes.finish().md5_hex, parts.len());
    let checksum = match checksum {
        ObjectChecksum::FullObject(algorithm) => full_object(&parts, algorithm)?,
        ObjectChecksum::Composite(algorithm) => composite(&parts, algorithm)?,
    };
    Ok(Assembled {
        etag,
        size,
        checksum,
    })
}

/// The whole-object CRC, combined part by part.
fn full_object(
    parts: &[(u16, &PartRecord)],
    algorithm: ChecksumAlgorithm,
) -> Result<String, Refusal> {
    let mut whole: Option<(u64, usize)> = None;
    for (number, part) in parts {
        let bytes = raw(part, algorithm).ok_or(Refusal::Part(*number))?;
        let value = crc_value(&bytes).ok_or(Refusal::Part(*number))?;
        whole = Some(match whole {
            None => (value, bytes.len()),
            Some((sum, width)) => (
                combine(algorithm, sum, value, part.size).ok_or(Refusal::Part(*number))?,
                width,
            ),
        });
    }
    let (value, width) = whole.ok_or(Refusal::Count)?;
    let bytes = value.to_be_bytes();
    let start = bytes.len().checked_sub(width).ok_or(Refusal::Count)?;
    Ok(STANDARD.encode(&bytes[start..]))
}

/// The algorithm over the parts' raw checksums in order, `-N`; the parts must be numbered 1 to N.
fn composite(
    parts: &[(u16, &PartRecord)],
    algorithm: ChecksumAlgorithm,
) -> Result<String, Refusal> {
    let mut concatenated = Vec::new();
    for (position, (number, part)) in (1_u16..).zip(parts) {
        if *number != position {
            return Err(Refusal::Part(position));
        }
        concatenated.extend(raw(part, algorithm).ok_or(Refusal::Part(*number))?);
    }
    let mut hashes = Hashes::new();
    hashes.update(&concatenated);
    Ok(format!(
        "{}-{}",
        hashes.finish().value(algorithm),
        parts.len()
    ))
}

#[cfg(test)]
#[path = "multipart_tests.rs"]
mod tests;
