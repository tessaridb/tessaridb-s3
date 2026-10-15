use std::collections::{BTreeMap, BTreeSet};

use tessari_s3_types::{Code, NodeId, Zone};

use super::{PlacementKey, placement};
use crate::erasure::{Layout, LayoutNode};

const SECRET: &[u8] = b"cluster-secret-0123456789abcdef0123";

fn members(names: &[&str]) -> Vec<LayoutNode> {
    names
        .iter()
        .map(|name| LayoutNode::new(NodeId::new(name).expect("node id"), None))
        .collect()
}

fn layout() -> Layout {
    Layout::new(
        1,
        Code::new(4, 2).expect("code"),
        vec![members(&["a", "b", "c", "d", "e", "f"])],
    )
    .expect("layout")
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
fn a_one_set_layout_places_every_id_where_it_always_did() {
    // Recorded from the single-set placement before layouts held several sets: data already on the drives is found
    // where it was written.
    let golden = ["b", "c", "b", "f", "f", "d", "d", "d", "e", "e", "c", "a"];
    let layout = layout();
    let key = PlacementKey::derive(SECRET);
    for (n, first) in (0_u16..).zip(golden) {
        assert_eq!(placement(&key, id(n), &layout)[0].as_str(), first, "id {n}");
    }
}

#[test]
fn every_shard_lands_on_its_own_node_in_the_sets_rotated_order() {
    let layout = layout();
    let key = PlacementKey::derive(SECRET);
    let order: Vec<String> = ["a", "b", "c", "d", "e", "f"].map(str::to_owned).to_vec();
    for n in 0..64 {
        let placed = names(&placement(&key, id(n), &layout));
        assert_eq!(placed.len(), 6);
        let start = order
            .iter()
            .position(|name| *name == placed[0])
            .expect("a layout node");
        let rotated: Vec<String> = order.iter().cycle().skip(start).take(6).cloned().collect();
        assert_eq!(placed, rotated, "id {n}: the set's order, rotated");
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

#[test]
fn an_id_goes_to_one_whole_set_and_ids_spread_over_every_set() {
    let sets = [
        ["a", "b", "c", "d", "e", "f"],
        ["g", "h", "i", "j", "k", "l"],
        ["m", "n", "o", "p", "q", "r"],
    ];
    let layout = Layout::new(
        2,
        Code::new(4, 2).expect("code"),
        sets.iter().map(|set| members(set)).collect(),
    )
    .expect("three sets");
    let key = PlacementKey::derive(SECRET);
    let mut per_set: BTreeMap<usize, usize> = BTreeMap::new();
    let mut firsts: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for n in 0..600 {
        let placed = names(&placement(&key, id(n), &layout));
        let set = sets
            .iter()
            .position(|set| set.contains(&placed[0].as_str()))
            .expect("a layout node");
        let mine: BTreeSet<&str> = sets[set].iter().copied().collect();
        let got: BTreeSet<&str> = placed.iter().map(String::as_str).collect();
        assert_eq!(got, mine, "id {n}: every shard in one set, one per node");
        bump(per_set.entry(set).or_default());
        firsts.entry(set).or_default().insert(placed[0].clone());
    }
    assert!(
        firsts.values().all(|nodes| nodes.len() == 6),
        "the rotation within a set does not follow from the set chosen: {firsts:?}"
    );
    assert_eq!(per_set.len(), 3, "{per_set:?}");
    assert!(
        per_set.values().all(|count| *count > 150),
        "uneven: {per_set:?}"
    );
}

/// Adds one to a counter that cannot get near its type's bound in these tests.
fn bump(count: &mut usize) {
    *count = count.checked_add(1).expect("small");
}

/// A small deterministic generator, so the sweep below covers many layouts and replays exactly.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let drawn = (self.0 >> 33)
            .checked_rem(u64::try_from(bound).expect("small"))
            .expect("a non-zero bound");
        usize::try_from(drawn).expect("small")
    }
}

#[test]
fn losing_any_one_zone_leaves_every_id_with_the_data_shards_it_needs() {
    // Whatever layout validation accepts, a whole zone lost costs every id at most m shards.
    let code = Code::new(4, 2).expect("code");
    let key = PlacementKey::derive(SECRET);
    let mut random = Lcg(7);
    let mut checked: usize = 0;
    for round in 0..200 {
        let zones = random.below(3).checked_add(3).expect("small");
        let sets = random.below(3).checked_add(1).expect("small");
        let candidates: Vec<LayoutNode> = (0..sets.checked_mul(6).expect("small"))
            .map(|n| {
                LayoutNode::new(
                    NodeId::new(&format!("n{round}-{n}")).expect("id"),
                    Some(Zone::new(&format!("z{}", random.below(zones))).expect("zone")),
                )
            })
            .collect();
        let Ok(layout) = Layout::new(
            1,
            code,
            candidates.chunks(6).map(<[LayoutNode]>::to_vec).collect(),
        ) else {
            continue;
        };
        bump(&mut checked);
        let zone_of: BTreeMap<&NodeId, &Zone> = candidates
            .iter()
            .map(|member| (member.node(), member.zone().expect("zoned")))
            .collect();
        for n in 0..32 {
            let mut per_zone: BTreeMap<&Zone, usize> = BTreeMap::new();
            for node in placement(&key, id(n), &layout) {
                bump(per_zone.entry(zone_of[node]).or_default());
            }
            assert!(
                per_zone
                    .values()
                    .all(|shards| *shards <= usize::from(code.parity())),
                "round {round} id {n}: {per_zone:?}"
            );
        }
    }
    assert!(checked > 20, "only {checked} random layouts were valid");
}
