//! [`NodeId`]: the name an S3 node is known by in the cluster's metadata.

use crate::{Error, Result};

/// The longest node id, in bytes.
const MAX_LEN: usize = 63;

/// A node's cluster name: 1 to 63 bytes of lowercase letters, digits and inner hyphens. It is used as a record id
/// and in paths of the internal surface, so nothing outside that alphabet can reach either.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(String);

impl NodeId {
    /// Checks `id` against the alphabet.
    ///
    /// # Errors
    /// [`Error::InvalidNodeId`].
    pub fn new(id: &str) -> Result<Self> {
        let alphanumeric = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
        let edges = id.bytes().next().is_some_and(alphanumeric)
            && id.bytes().next_back().is_some_and(alphanumeric);
        if id.len() > MAX_LEN || !edges || !id.bytes().all(|b| alphanumeric(b) || b == b'-') {
            return Err(Error::InvalidNodeId);
        }
        Ok(Self(id.to_owned()))
    }

    /// The id as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "node_id_tests.rs"]
mod tests;
