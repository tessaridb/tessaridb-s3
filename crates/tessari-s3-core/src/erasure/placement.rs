//! Placement: which node holds which shard of a data id. A keyed hash of the id — SipHash-1-3 keyed from the
//! cluster secret — picks one erasure set of the layout and rotates that set's node order, so parity load spreads
//! over every node, an id's placement is stable for as long as its layout is, and nobody without the secret can aim
//! ids at one set or one node.

use std::hash::Hasher;

use siphasher::sip::SipHasher13;
use tessari_s3_types::NodeId;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::{Layout, LayoutNode};

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

/// The node of each shard of data `id` under `layout`: shard j lives on element j of the answer. The keyed hash h
/// of the id picks the set (h mod sets) and the rotation of that set's order ((h div sets) mod width) — with one
/// set, h mod width, the placement every single-set layout has always used.
#[must_use]
pub fn placement<'a>(key: &PlacementKey, id: [u8; 16], layout: &'a Layout) -> Vec<&'a NodeId> {
    let mut hasher = SipHasher13::new_with_key(&key.0);
    hasher.write(&id);
    let hash = hasher.finish();
    let sets = layout.sets();
    // A layout holds at least one set of at least two nodes, so every division below has a non-zero divisor and
    // every remainder indexes inside its slice; the fallbacks are unreachable.
    let count = u64::try_from(sets.len()).unwrap_or(u64::MAX);
    let chosen = hash
        .checked_rem(count)
        .and_then(|set| usize::try_from(set).ok())
        .and_then(|set| sets.get(set))
        .map_or(&[][..], Vec::as_slice);
    let width = u64::try_from(chosen.len()).unwrap_or(u64::MAX);
    let start = hash
        .checked_div(count)
        .and_then(|turn| turn.checked_rem(width))
        .and_then(|start| usize::try_from(start).ok())
        .unwrap_or(0);
    chosen
        .iter()
        .map(LayoutNode::node)
        .cycle()
        .skip(start)
        .take(chosen.len())
        .collect()
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
