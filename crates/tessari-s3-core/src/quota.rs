//! Bucket quotas (ADR-0007): an optional limit on a bucket's bytes and on its objects, checked before a write adds to
//! it, against the last usage measurement plus what the write brings. Pure: everything it reads is in its arguments.

use crate::dispatch::Operation;

/// The operations that add bytes or objects to a bucket, and so are refused when they would pass its quota.
pub const ADDING: [Operation; 5] = [
    Operation::PutObject,
    Operation::CopyObject,
    Operation::UploadPart,
    Operation::UploadPartCopy,
    Operation::CompleteMultipartUpload,
];

/// Whether `operation` adds to the bucket it writes.
#[must_use]
pub fn adds_to_bucket(operation: Operation) -> bool {
    ADDING.contains(&operation)
}

/// A bucket's limits; `None` is no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quota {
    /// The most bytes the bucket may hold.
    pub max_bytes: Option<u64>,
    /// The most objects the bucket may hold.
    pub max_objects: Option<u64>,
}

impl Quota {
    /// Whether any limit is set — a bucket without one needs no measurement read.
    #[must_use]
    pub const fn limits(&self) -> bool {
        self.max_bytes.is_some() || self.max_objects.is_some()
    }
}

/// What a bucket held when it was last measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    /// Objects.
    pub objects: u64,
    /// Logical bytes.
    pub bytes: u64,
}

/// What a write brings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adding {
    /// Bytes it adds.
    pub bytes: u64,
    /// Whether it may add an object.
    pub object: bool,
}

/// Which limit a write would pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exceeded {
    /// The byte limit.
    Bytes,
    /// The object limit.
    Objects,
}

/// Admits the write unless `held` plus `adding` passes a limit of `quota`; reaching a limit exactly is admitted.
///
/// # Errors
/// The limit the write would pass.
pub fn admits(quota: Quota, held: Held, adding: Adding) -> Result<(), Exceeded> {
    if let Some(max) = quota.max_bytes
        && held.bytes.saturating_add(adding.bytes) > max
    {
        return Err(Exceeded::Bytes);
    }
    if let Some(max) = quota.max_objects
        && held.objects.saturating_add(u64::from(adding.object)) > max
    {
        return Err(Exceeded::Objects);
    }
    Ok(())
}
