//! What reading or changing one version's tags found (S3 ref 05 M8).

use std::collections::BTreeMap;

/// The outcome of a tagging request on one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagsOutcome {
    /// The version, its id, and its tags — after the change, for a change.
    Version {
        /// Its version id (`null` for a null version).
        version_id: String,
        /// Its tags by key.
        tags: BTreeMap<String, String>,
    },
    /// The bucket does not exist.
    NoSuchBucket,
    /// The key has no current object.
    NoSuchKey,
    /// No version of the key has that id.
    NoSuchVersion,
    /// No version this server issues is spelled so.
    Malformed,
    /// The version named is a delete marker, which carries no tags.
    Marker,
}
