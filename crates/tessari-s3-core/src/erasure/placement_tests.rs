use std::collections::BTreeSet;

use tessari_s3_types::{Code, NodeId};

use super::{PlacementKey, placement};
use crate::erasure::Layout;

const SECRET: &[u8] = b"cluster-secret-0123456789abcdef0123";

fn layout() -> Layout {
    let nodes = ["a", "b", "c", "d", "e", "f"]
        .into_iter()
        .map(|name| NodeId::new(name).expect("node id"))
        .collect();
    Layout::new(1, Code::new(4, 2).expect("code"), nodes).expect("layout")
}

fn id(n: u16) -> [u8; 16] {
    let mut id = [0_u8; 16];
    id[..2].copy_from_slice(&n.to_be_bytes());
    id
}

fn names(placed: &[&NodeId]) -> Vec<String> {
    placed.iter().map(|node| node.as_str().to_owned()).collect()
}

#[test]
fn every_shard_lands_on_its_own_node_in_the_layouts_rotated_order() {
    let layout = layout();
    let key = PlacementKey::derive(SECRET);
    let order: Vec<String> = names(&layout.nodes().iter().collect::<Vec<_>>());
    for n in 0..64 {
        let placed = names(&placement(&key, id(n), &layout));
        assert_eq!(placed.len(), 6);
        let distinct: BTreeSet<&String> = placed.iter().collect();
        assert_eq!(distinct.len(), 6, "id {n}: one shard per node");
        let start = order
            .iter()
            .position(|name| *name == placed[0])
            .expect("a layout node");
        let rotated: Vec<String> = order.iter().cycle().skip(start).take(6).cloned().collect();
        assert_eq!(placed, rotated, "id {n}: the layout order, rotated");
        assert_eq!(
            placed,
            names(&placement(&key, id(n), &layout)),
            "id {n}: stable"
        );
    }
}

#[test]
fn ids_spread_their_first_shard_over_every_node() {
    let layout = layout();
    let key = PlacementKey::derive(SECRET);
    let firsts: BTreeSet<String> = (0..256)
        .map(|n| placement(&key, id(n), &layout)[0].as_str().to_owned())
        .collect();
    assert_eq!(firsts.len(), 6, "{firsts:?}");
}

#[test]
fn another_secret_places_ids_differently() {
    let layout = layout();
    let ours = PlacementKey::derive(SECRET);
    let theirs = PlacementKey::derive(b"another-secret-0123456789abcdef0123");
    let differing = (0..64)
        .filter(|n| {
            names(&placement(&ours, id(*n), &layout)) != names(&placement(&theirs, id(*n), &layout))
        })
        .count();
    assert!(differing > 32, "only {differing} of 64 ids moved");
}
