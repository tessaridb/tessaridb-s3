//! A multipart upload and its parts as the rest of the server sees them, and what each operation ended in.

use std::collections::BTreeMap;

use tessari_s3_core::objects::checksum::ChecksumAlgorithm;
use tessari_s3_core::objects::multipart::{ListedPart, Refusal};
use tessari_s3_types::{PartNumber, Timestamp, UploadId};

use crate::objects::{WriteCondition, Written};

/// What CreateMultipartUpload fixes for the object the upload will become.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewUpload {
    /// Stored system headers, lowercase names.
    pub headers: BTreeMap<String, String>,
    /// User metadata, lowercase names without the `x-amz-meta-` prefix.
    pub metadata: BTreeMap<String, String>,
    /// The checksum algorithm the object will carry (`CRC32` …), when the client declared one.
    pub checksum_algorithm: Option<String>,
    /// `COMPOSITE` or `FULL_OBJECT`, when the client declared one.
    pub checksum_type: Option<String>,
}

/// One uploaded part ready to commit: its bytes are already in data file `data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPart {
    /// The data file holding the part.
    pub data: [u8; 16],
    /// Size in bytes.
    pub size: u64,
    /// The quoted ETag (the MD5 of the part).
    pub etag: String,
    /// Checksums by algorithm name, base64.
    pub checksums: BTreeMap<String, String>,
}

/// A stored part, as ListParts shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPart {
    /// The part number.
    pub number: PartNumber,
    /// Size in bytes.
    pub size: u64,
    /// The quoted ETag.
    pub etag: String,
    /// When it was uploaded.
    pub modified: Timestamp,
    /// Checksums by algorithm name.
    pub checksums: BTreeMap<String, String>,
}

/// What CreateMultipartUpload ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Created {
    /// The new upload.
    Created(UploadId),
    /// The bucket does not exist.
    NoSuchBucket,
}

/// What an UploadPart commit ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartWritten {
    /// The part is stored.
    Committed,
    /// The upload does not exist for this bucket and key (never did, aborted, or completed).
    NoSuchUpload,
    /// The part's data file was reclaimed before the commit; nothing was written.
    DataReclaimed,
}

/// One ListParts page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartsPage {
    /// Parts in ascending number.
    pub parts: Vec<StoredPart>,
    /// Whether more parts follow.
    pub truncated: bool,
}

/// What a CompleteMultipartUpload asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The parts that form the object, as listed.
    pub parts: Vec<ListedPart>,
    /// The condition the object is written under.
    pub condition: WriteCondition,
    /// A checksum of the whole object the client sent with the request, to be checked against the computed one.
    pub checksum: Option<(ChecksumAlgorithm, String)>,
    /// The object size the client declared (`x-amz-mp-object-size`).
    pub size: Option<u64>,
}

/// What a CompleteMultipartUpload ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completed {
    /// The object is visible.
    Done {
        /// Its quoted multipart ETag.
        etag: String,
        /// Its checksum algorithm and value.
        checksum: (ChecksumAlgorithm, String),
        /// Whether the checksum is composite (otherwise it covers the whole object).
        composite: bool,
    },
    /// The upload does not exist for this bucket and key (never did, aborted, or completed).
    NoSuchUpload,
    /// The listed parts cannot form the object.
    Refused(Refusal),
    /// The declared object size is not the parts' total.
    SizeMismatch,
    /// The whole-object checksum the client sent is not the computed one.
    ChecksumMismatch,
    /// The object write did not commit, for the reason given (bucket gone, condition false).
    NotWritten(Written),
}
