//! A layout: which nodes hold the shards of the objects written under it — one or more erasure sets of k + m nodes,
//! no set holding more shards in one failure domain than the code survives losing. Versioned, so an object records
//! the layout it was written with and is found after the cluster's layout moves on.

use std::collections::BTreeMap;

use tessari_s3_types::{NodeId, Zone};

use super::Code;

/// Why a layout was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// Version 0: versions start at 1.
    #[error("a layout version starts at 1")]
    Version,
    /// No erasure set at all.
    #[error("a layout holds at least one erasure set")]
    NoSets,
    /// A set without one node per shard.
    #[error("set {set} names {nodes} nodes for a code of {width} shards")]
    Width {
        /// The set, counted from 0.
        set: usize,
        /// Nodes named.
        nodes: usize,
        /// Shards the code cuts an object into.
        width: u8,
    },
    /// A node named twice — in one set (two shards on one node) or in two sets.
    #[error("a node is named twice in one layout")]
    Duplicate,
    /// A set placing more shards in one failure domain than the code survives losing.
    #[error(
        "set {set} places {shards} shards in zone {zone}; losing it must cost at most {parity}"
    )]
    ZoneOverloaded {
        /// The set, counted from 0.
        set: usize,
        /// The zone.
        zone: String,
        /// Shards of the set in it.
        shards: usize,
        /// Parity shards: the most the set may lose.
        parity: u8,
    },
}

/// A node of a layout and the failure domain it was in when the layout was made; a node with no zone is a domain
/// of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutNode {
    node: NodeId,
    zone: Option<Zone>,
}

impl LayoutNode {
    /// `node`, in `zone` when it names one.
    #[must_use]
    pub const fn new(node: NodeId, zone: Option<Zone>) -> Self {
        Self { node, zone }
    }

    /// The node.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    /// Its zone, when it names one.
    #[must_use]
    pub const fn zone(&self) -> Option<&Zone> {
        self.zone.as_ref()
    }
}

/// Version `version` of the cluster's layout: an object goes to one set, and shard i of it (before its rotation)
/// to node i of that set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    version: u32,
    code: Code,
    sets: Vec<Vec<LayoutNode>>,
}

impl Layout {
    /// A layout as stored or staged.
    ///
    /// # Errors
    /// [`LayoutError`] for version 0, no set, a set of other than k + m nodes, a node named twice, or a set with
    /// more than m shards in one zone.
    pub fn new(version: u32, code: Code, sets: Vec<Vec<LayoutNode>>) -> Result<Self, LayoutError> {
        if version == 0 {
            return Err(LayoutError::Version);
        }
        if sets.is_empty() {
            return Err(LayoutError::NoSets);
        }
        for (index, set) in sets.iter().enumerate() {
            if set.len() != usize::from(code.width()) {
                return Err(LayoutError::Width {
                    set: index,
                    nodes: set.len(),
                    width: code.width(),
                });
            }
            if let Some((zone, shards)) = crowded(set, code.parity()) {
                return Err(LayoutError::ZoneOverloaded {
                    set: index,
                    zone,
                    shards,
                    parity: code.parity(),
                });
            }
        }
        let mut distinct: Vec<&NodeId> = sets.iter().flatten().map(LayoutNode::node).collect();
        let named = distinct.len();
        distinct.sort();
        distinct.dedup();
        if distinct.len() != named {
            return Err(LayoutError::Duplicate);
        }
        Ok(Self {
            version,
            code,
            sets,
        })
    }

    /// The first layout of a cluster: one set filled from the registered nodes in id order, each taken unless its
    /// zone already holds m shards of the set — so every node computing it from the same registrations proposes the
    /// same one. `None` until enough nodes in enough zones have registered.
    #[must_use]
    pub fn first(code: Code, registered: impl IntoIterator<Item = LayoutNode>) -> Option<Self> {
        let mut nodes: Vec<LayoutNode> = registered.into_iter().collect();
        nodes.sort_by(|left, right| left.node.cmp(&right.node));
        nodes.dedup_by(|left, right| left.node == right.node);
        let mut set: Vec<LayoutNode> = Vec::with_capacity(usize::from(code.width()));
        for candidate in nodes {
            if set.len() == usize::from(code.width()) {
                break;
            }
            let shared = set
                .iter()
                .filter(|member| member.zone.is_some() && member.zone == candidate.zone)
                .count();
            if shared < usize::from(code.parity()) {
                set.push(candidate);
            }
        }
        Self::new(1, code, vec![set]).ok()
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

    /// The erasure sets, each one node per shard.
    #[must_use]
    pub fn sets(&self) -> &[Vec<LayoutNode>] {
        &self.sets
    }
}

/// The zone of `set` holding the most shards and how many, when that is more than `parity`; nodes without a zone are
/// domains of their own and a node named twice is [`LayoutError::Duplicate`], so they never crowd one.
fn crowded(set: &[LayoutNode], parity: u8) -> Option<(String, usize)> {
    let mut per_zone: BTreeMap<&Zone, usize> = BTreeMap::new();
    for zone in set.iter().filter_map(LayoutNode::zone) {
        let shards = per_zone.entry(zone).or_default();
        *shards = shards.saturating_add(1);
    }
    per_zone
        .into_iter()
        .filter(|(_, shards)| *shards > usize::from(parity))
        .max_by_key(|(_, shards)| *shards)
        .map(|(zone, shards)| (zone.as_str().to_owned(), shards))
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
