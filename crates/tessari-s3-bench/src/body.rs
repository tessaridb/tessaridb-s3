//! Object bodies: deterministic per key, so a GET can be checked against what the PUT sent without keeping it.

use std::fmt::Write as _;

use bytes::Bytes;
use sha2::{Digest, Sha256};
use tessari_s3_constants::{BENCH_MULTIPART_PARTS, BENCH_PART_BYTES};

/// The object sizes the benchmark measures.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum SizeClass {
    /// 4 KiB.
    #[serde(rename = "4KiB")]
    Small,
    /// 128 KiB.
    #[serde(rename = "128KiB")]
    Medium,
    /// 1 MiB.
    #[serde(rename = "1MiB")]
    Large,
    /// 64 MiB.
    #[serde(rename = "64MiB")]
    Huge,
    /// 256 MiB uploaded as 16 parts of 16 MiB.
    #[serde(rename = "multipart-256MiB")]
    Multipart,
}

impl SizeClass {
    /// Parses a class as the configuration spells it: `4k`, `128k`, `1m`, `64m` or `multipart`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "4k" => Some(Self::Small),
            "128k" => Some(Self::Medium),
            "1m" => Some(Self::Large),
            "64m" => Some(Self::Huge),
            "multipart" => Some(Self::Multipart),
            _ => None,
        }
    }

    /// The name used in keys.
    #[must_use]
    pub const fn key_part(self) -> &'static str {
        match self {
            Self::Small => "4k",
            Self::Medium => "128k",
            Self::Large => "1m",
            Self::Huge => "64m",
            Self::Multipart => "multipart",
        }
    }

    /// The object size in bytes.
    #[must_use]
    pub fn bytes(self) -> u64 {
        match self {
            Self::Small => 4 * 1024,
            Self::Medium => 128 * 1024,
            Self::Large => 1024 * 1024,
            Self::Huge => 64 * 1024 * 1024,
            Self::Multipart => BENCH_PART_BYTES.saturating_mul(u64::from(BENCH_MULTIPART_PARTS)),
        }
    }
}

/// One request body, ready to send: the bytes, the SHA-256 the request declares, and the BLAKE3 the GET checks.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The bytes.
    pub body: Bytes,
    /// Lowercase hex SHA-256, for `x-amz-content-sha256`.
    pub sha256_hex: String,
}

/// The bytes of `key`'s part `part` (0 for a single-part object), `len` long: a BLAKE3 output stream keyed by both.
#[must_use]
pub fn prepare(key: &str, part: u32, len: usize) -> Prepared {
    let mut body = vec![0_u8; len];
    blake3::Hasher::new_derive_key("tessaridb-s3 bench body")
        .update(key.as_bytes())
        .update(&part.to_le_bytes())
        .finalize_xof()
        .fill(&mut body);
    Prepared {
        sha256_hex: sha256_hex(&body),
        body: Bytes::from(body),
    }
}

/// Lowercase hex SHA-256 of `bytes`, as `x-amz-content-sha256` declares a payload.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        // Writing to a String cannot fail.
        let _written = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::{SizeClass, prepare};

    #[test]
    fn a_body_is_the_same_for_its_key_and_part_and_differs_otherwise() {
        let first = prepare("p000/4k/c1/00000000", 0, 4096);
        assert_eq!(first.body, prepare("p000/4k/c1/00000000", 0, 4096).body);
        assert_ne!(first.body, prepare("p000/4k/c1/00000001", 0, 4096).body);
        assert_ne!(first.body, prepare("p000/4k/c1/00000000", 1, 4096).body);
        assert_eq!(first.body.len(), 4096);
        // The declared hash is the body's own SHA-256, as the server will recompute it.
        let expected: String = Sha256::digest(&first.body)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(first.sha256_hex, expected);
    }

    #[test]
    fn classes_parse_from_their_configuration_names_and_have_their_sizes() {
        for (name, bytes) in [
            ("4k", 4096),
            ("128k", 131_072),
            ("1m", 1_048_576),
            ("64m", 67_108_864),
            ("multipart", 268_435_456),
        ] {
            let class = SizeClass::parse(name).expect(name);
            assert_eq!((class.key_part(), class.bytes()), (name, bytes));
        }
        assert_eq!(SizeClass::parse("2m"), None);
    }
}
