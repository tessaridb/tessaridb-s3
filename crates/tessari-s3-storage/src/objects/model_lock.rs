//! What Object Lock keeps a version under (ADR-0016): as a write asks for it, as stored, and what changing it found.

use tessari_s3_core::object_lock::Mode;
use tessari_s3_types::Timestamp;

/// Until when a new version is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainUntil {
    /// The instant the request named.
    At(Timestamp),
    /// The bucket's default: this many seconds after the commit, by the metadata store's clock.
    AfterCommit(u64),
}

/// The Object Lock settings a new version is written with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lock {
    /// Its retention: a mode and until when.
    pub retention: Option<(Mode, RetainUntil)>,
    /// Whether it is written under a legal hold.
    pub legal_hold: bool,
}

/// A version's Object Lock as stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VersionLock {
    /// Its retention: a mode and the instant it is kept until.
    pub retention: Option<(Mode, Timestamp)>,
    /// Whether a legal hold is on.
    pub legal_hold: bool,
}

/// What reading or changing a version's Object Lock found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockOutcome {
    /// The version, its id, and its lock — after the change, for a change.
    Version {
        /// Its version id (`null` for a null version).
        version_id: String,
        /// Its lock.
        lock: VersionLock,
    },
    /// The bucket does not exist.
    NoSuchBucket,
    /// The key has no current object.
    NoSuchKey,
    /// No version of the key has that id.
    NoSuchVersion,
    /// No version this server issues is spelled so.
    Malformed,
    /// The version named is a delete marker, which carries no lock.
    Marker,
    /// The change would shorten or remove a retention that keeps the version (COMPLIANCE, or GOVERNANCE without a
    /// bypass): refused, nothing changed.
    Locked,
}
