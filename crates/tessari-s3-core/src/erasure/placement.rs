//! Placement: which node holds which shard of a data id. The layout's node order is rotated per id by SipHash-1-3
//! keyed from the cluster secret, so parity load spreads over every node, an id's placement is stable for as long
//! as its layout is, and nobody without the secret can aim ids at one node.

use std::hash::Hasher;

use siphasher::sip::SipHasher13;
use tessari_s3_types::NodeId;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::Layout;

/// The context the placement key is derived under; another scheme gets another context, never a reinterpretation.
const PLACEMENT_CONTEXT: &str = "tessaridb-s3 placement v1";

/// The 128-bit key placement hashes with, derived from the cluster secret; scrubbed when dropped.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct PlacementKey([u8; 16]);

impl PlacementKey {
    /// The key for the cluster whose secret is `secret`.
    #[must_use]
    pub fn derive(secret: &[u8]) -> Self {
        let mut derived = blake3::derive_key(PLACEMENT_CONTEXT, secret);
        let mut key = [0_u8; 16];
        key.copy_from_slice(&derived[..16]);
        derived.zeroize();
        Self(key)
    }
}

/// The node of each shard of data `id` under `layout`: shard j lives on element j of the answer — the layout's
/// order, rotated by the keyed hash of the id.
#[must_use]
pub fn placement<'a>(key: &PlacementKey, id: [u8; 16], layout: &'a Layout) -> Vec<&'a NodeId> {
    let nodes = layout.nodes();
    let mut hasher = SipHasher13::new_with_key(&key.0);
    hasher.write(&id);
    let width = u64::try_from(nodes.len()).unwrap_or(u64::MAX);
    // A layout holds at least two nodes, so the remainder exists and is below the width.
    let start = hasher
        .finish()
        .checked_rem(width)
        .and_then(|start| usize::try_from(start).ok())
        .unwrap_or(0);
    nodes.iter().cycle().skip(start).take(nodes.len()).collect()
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
