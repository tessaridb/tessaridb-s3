//! A layout: which nodes hold the shards of the objects written under it. Versioned, so an object records the
//! layout it was written with and is found after the cluster's layout moves on.

use tessari_s3_types::NodeId;

use super::Code;

/// Why a layout was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// Version 0: versions start at 1.
    #[error("a layout version starts at 1")]
    Version,
    /// Not one node per shard.
    #[error("{nodes} nodes for a code of {width} shards")]
    Width {
        /// Nodes named.
        nodes: usize,
        /// Shards the code cuts an object into.
        width: u8,
    },
    /// A node named twice: two shards on one node would be one failure domain.
    #[error("a node holds two shards of one layout")]
    Duplicate,
}

/// Version `version` of the cluster's layout: shard i of an object (before its rotation) lives on `nodes[i]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    version: u32,
    code: Code,
    nodes: Vec<NodeId>,
}

impl Layout {
    /// A layout as stored.
    ///
    /// # Errors
    /// [`LayoutError`] for version 0, a node count other than k + m, or a node named twice.
    pub fn new(version: u32, code: Code, nodes: Vec<NodeId>) -> Result<Self, LayoutError> {
        if version == 0 {
            return Err(LayoutError::Version);
        }
        if nodes.len() != usize::from(code.width()) {
            return Err(LayoutError::Width {
                nodes: nodes.len(),
                width: code.width(),
            });
        }
        let mut distinct = nodes.clone();
        distinct.sort();
        distinct.dedup();
        if distinct.len() != nodes.len() {
            return Err(LayoutError::Duplicate);
        }
        Ok(Self {
            version,
            code,
            nodes,
        })
    }

    /// The first layout of a cluster: the k + m lowest distinct registered ids, so every node computing it from the
    /// same registrations proposes the same one. `None` until enough nodes have registered.
    #[must_use]
    pub fn first(code: Code, registered: impl IntoIterator<Item = NodeId>) -> Option<Self> {
        let mut nodes: Vec<NodeId> = registered.into_iter().collect();
        nodes.sort();
        nodes.dedup();
        nodes.truncate(usize::from(code.width()));
        Self::new(1, code, nodes).ok()
    }

    /// The version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// The code objects are written with under this layout.
    #[must_use]
    pub const fn code(&self) -> Code {
        self.code
    }

    /// The nodes, one per shard.
    #[must_use]
    pub fn nodes(&self) -> &[NodeId] {
        &self.nodes
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
