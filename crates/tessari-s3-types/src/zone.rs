//! [`Zone`]: the failure domain a node belongs to — a zone, a rack, a room; whatever the operator names it, the
//! layout never puts more shards of one object in it than the code survives losing.

use crate::node_id::short_name;
use crate::{Error, Result};

/// The longest zone name, in bytes.
const MAX_LEN: usize = 32;

/// A failure domain's name: 1 to 32 bytes of lowercase letters, digits and inner hyphens.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Zone(String);

impl Zone {
    /// Checks `zone` against the alphabet.
    ///
    /// # Errors
    /// [`Error::InvalidZone`].
    pub fn new(zone: &str) -> Result<Self> {
        if !short_name(zone, MAX_LEN) {
            return Err(Error::InvalidZone);
        }
        Ok(Self(zone.to_owned()))
    }

    /// The name as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "zone_tests.rs"]
mod tests;
