//! A multipart upload and its parts as the rest of the server sees them, and what each operation ended in.

use std::collections::BTreeMap;

use tessari_s3_types::{PartNumber, Timestamp, UploadId};

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
