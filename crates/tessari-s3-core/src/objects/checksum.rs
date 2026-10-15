//! Object integrity values: the MD5 behind a single-part ETag, and the `x-amz-checksum-*` algorithms a client may
//! declare. Each is computed while the body streams, compared before anything commits, and stored with the object.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use md5::{Digest as _, Md5};
use sha1::Sha1;
use sha2::Sha256;

/// The checksum algorithms this server validates and stores. SHA512, MD5 and the XXHASH family exist in the S3 model
/// and are refused rather than accepted and dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChecksumAlgorithm {
    /// CRC-32 (ISO-HDLC), `x-amz-checksum-crc32`.
    Crc32,
    /// CRC-32C (Castagnoli), `x-amz-checksum-crc32c`.
    Crc32c,
    /// CRC-64/NVME, `x-amz-checksum-crc64nvme`; the default stored when a client declares none.
    Crc64Nvme,
    /// SHA-1, `x-amz-checksum-sha1`.
    Sha1,
    /// SHA-256, `x-amz-checksum-sha256`.
    Sha256,
}

/// What a checksum header names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declared {
    /// An algorithm this server implements.
    Supported(ChecksumAlgorithm),
    /// An algorithm in the S3 model this server does not implement.
    Unsupported,
}

impl ChecksumAlgorithm {
    /// Every supported algorithm.
    pub const ALL: [Self; 5] = [
        Self::Crc32,
        Self::Crc32c,
        Self::Crc64Nvme,
        Self::Sha1,
        Self::Sha256,
    ];

    /// The model's name: `CRC32`, `CRC32C`, `CRC64NVME`, `SHA1`, `SHA256`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Crc32 => "CRC32",
            Self::Crc32c => "CRC32C",
            Self::Crc64Nvme => "CRC64NVME",
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
        }
    }

    /// The header carrying its value: `x-amz-checksum-crc32` …
    #[must_use]
    pub const fn header(self) -> &'static str {
        match self {
            Self::Crc32 => "x-amz-checksum-crc32",
            Self::Crc32c => "x-amz-checksum-crc32c",
            Self::Crc64Nvme => "x-amz-checksum-crc64nvme",
            Self::Sha1 => "x-amz-checksum-sha1",
            Self::Sha256 => "x-amz-checksum-sha256",
        }
    }

    /// The algorithm an `x-amz-checksum-<alg>` header (lowercase) or an algorithm name (any case) names, or `None`
    /// for text that names no checksum at all.
    #[must_use]
    pub fn parse(text: &str) -> Option<Declared> {
        let lower = text.to_ascii_lowercase();
        let name = lower.strip_prefix("x-amz-checksum-").unwrap_or(&lower);
        if let Some(algorithm) = Self::ALL
            .into_iter()
            .find(|a| a.name().eq_ignore_ascii_case(name))
        {
            return Some(Declared::Supported(algorithm));
        }
        // The rest of the model's ChecksumAlgorithm enum.
        matches!(
            name,
            "sha512" | "md5" | "xxhash64" | "xxhash3" | "xxhash128"
        )
        .then_some(Declared::Unsupported)
    }
}

/// Computes every supported checksum and the MD5 at once, so a body is hashed in one pass whatever was declared.
#[derive(Clone)]
pub struct Hashes {
    md5: Md5,
    crc32: crc_fast::Digest,
    crc32c: crc_fast::Digest,
    crc64nvme: crc_fast::Digest,
    sha1: Sha1,
    sha256: Sha256,
}

/// The finished values: the MD5 as lowercase hex (the ETag's content), each checksum as base64 of its big-endian
/// digest (the header's form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digests {
    /// Lowercase hex MD5.
    pub md5_hex: String,
    /// The raw MD5, for Content-MD5 comparison and multipart ETags.
    pub md5: [u8; 16],
    /// The raw SHA-256, for the signed payload hash of a body read in one pass.
    pub sha256: [u8; 32],
    values: [(ChecksumAlgorithm, String); 5],
}

impl Digests {
    /// The base64 value of `algorithm`.
    #[must_use]
    pub fn value(&self, algorithm: ChecksumAlgorithm) -> &str {
        self.values
            .iter()
            .find(|(a, _)| *a == algorithm)
            .map_or("", |(_, v)| v.as_str())
    }
}

impl Default for Hashes {
    fn default() -> Self {
        Self::new()
    }
}

impl Hashes {
    /// Fresh state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            md5: Md5::new(),
            crc32: crc_fast::Digest::new(crc_fast::CrcAlgorithm::Crc32IsoHdlc),
            crc32c: crc_fast::Digest::new(crc_fast::CrcAlgorithm::Crc32Iscsi),
            crc64nvme: crc_fast::Digest::new(crc_fast::CrcAlgorithm::Crc64Nvme),
            sha1: Sha1::new(),
            sha256: Sha256::new(),
        }
    }

    /// Feeds the next bytes of the body.
    pub fn update(&mut self, bytes: &[u8]) {
        self.md5.update(bytes);
        self.crc32.update(bytes);
        self.crc32c.update(bytes);
        self.crc64nvme.update(bytes);
        self.sha1.update(bytes);
        self.sha256.update(bytes);
    }

    /// The finished digests.
    #[must_use]
    pub fn finish(self) -> Digests {
        let md5: [u8; 16] = self.md5.finalize().into();
        let crc32 = u32::try_from(self.crc32.finalize())
            .unwrap_or_default()
            .to_be_bytes();
        let crc32c = u32::try_from(self.crc32c.finalize())
            .unwrap_or_default()
            .to_be_bytes();
        let crc64nvme = self.crc64nvme.finalize().to_be_bytes();
        let sha1 = self.sha1.finalize();
        let sha256 = self.sha256.finalize();
        Digests {
            md5_hex: md5.iter().map(|b| format!("{b:02x}")).collect(),
            md5,
            sha256: sha256.into(),
            values: [
                (ChecksumAlgorithm::Crc32, STANDARD.encode(crc32)),
                (ChecksumAlgorithm::Crc32c, STANDARD.encode(crc32c)),
                (ChecksumAlgorithm::Crc64Nvme, STANDARD.encode(crc64nvme)),
                (ChecksumAlgorithm::Sha1, STANDARD.encode(sha1)),
                (ChecksumAlgorithm::Sha256, STANDARD.encode(sha256)),
            ],
        }
    }
}

#[cfg(test)]
#[path = "checksum_tests.rs"]
mod tests;
