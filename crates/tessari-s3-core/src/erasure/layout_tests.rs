use tessari_s3_types::{NodeId, Zone};

use super::{Layout, LayoutError, LayoutNode};
use crate::erasure::Code;

fn node(id: &str, zone: Option<&str>) -> LayoutNode {
    LayoutNode::new(
        NodeId::new(id).expect("an id"),
        zone.map(|zone| Zone::new(zone).expect("a zone")),
    )
}

fn plain(ids: &[&str]) -> Vec<LayoutNode> {
    ids.iter().map(|id| node(id, None)).collect()
}

fn zoned(pairs: &[(&str, &str)]) -> Vec<LayoutNode> {
    pairs
        .iter()
        .map(|(id, zone)| node(id, Some(zone)))
        .collect()
}

fn names(layout: &Layout) -> Vec<Vec<&str>> {
    layout
        .sets()
        .iter()
        .map(|set| set.iter().map(|member| member.node().as_str()).collect())
        .collect()
}

fn code(data: u8, parity: u8) -> Code {
    Code::new(data, parity).expect("a code")
}

#[test]
fn the_first_layout_is_the_lowest_ids_whoever_computes_it() {
    let registered = plain(&["n4", "n2", "n9", "n1", "n2"]);
    let layout = Layout::first(code(2, 1), registered.clone()).expect("enough nodes");
    assert_eq!(
        (layout.version(), names(&layout)),
        (1, vec![vec!["n1", "n2", "n4"]])
    );
    let mut reversed = registered;
    reversed.reverse();
    assert_eq!(
        Layout::first(code(2, 1), reversed),
        Some(layout),
        "order of discovery does not matter"
    );
}

#[test]
fn the_first_layout_skips_a_node_whose_zone_is_full() {
    // 2+1 survives one shard per zone: n2 shares n1's zone, so n3 takes its place.
    let registered = zoned(&[("n1", "a"), ("n2", "a"), ("n3", "b"), ("n4", "c")]);
    let layout = Layout::first(code(2, 1), registered).expect("enough zones");
    assert_eq!(names(&layout), vec![vec!["n1", "n3", "n4"]]);
}

#[test]
fn too_few_distinct_nodes_or_zones_make_no_layout() {
    assert_eq!(Layout::first(code(2, 1), plain(&["n1", "n2", "n2"])), None);
    assert_eq!(
        Layout::first(code(2, 1), zoned(&[("n1", "a"), ("n2", "a"), ("n3", "b")])),
        None,
        "three nodes in two zones cannot hold 2+1 one shard per zone"
    );
}

#[test]
fn a_stored_layout_is_checked_before_it_is_believed() {
    let two_one = code(2, 1);
    assert!(Layout::new(3, two_one, vec![plain(&["a", "b", "c"])]).is_ok());
    assert_eq!(
        Layout::new(0, two_one, vec![plain(&["a", "b", "c"])]),
        Err(LayoutError::Version)
    );
    assert_eq!(
        Layout::new(1, two_one, Vec::new()),
        Err(LayoutError::NoSets)
    );
    assert_eq!(
        Layout::new(
            1,
            two_one,
            vec![plain(&["a", "b", "c"]), plain(&["d", "e"])]
        ),
        Err(LayoutError::Width {
            set: 1,
            nodes: 2,
            width: 3
        })
    );
    assert_eq!(
        Layout::new(1, two_one, vec![plain(&["a", "b", "a"])]),
        Err(LayoutError::Duplicate),
        "two shards of one set on one node"
    );
    assert_eq!(
        Layout::new(
            1,
            two_one,
            vec![plain(&["a", "b", "c"]), plain(&["d", "e", "a"])]
        ),
        Err(LayoutError::Duplicate),
        "one node in two sets"
    );
}

#[test]
fn a_set_holds_no_more_shards_in_one_zone_than_the_code_survives() {
    let four_two = code(4, 2);
    let spread = zoned(&[
        ("a", "z1"),
        ("b", "z1"),
        ("c", "z2"),
        ("d", "z2"),
        ("e", "z3"),
        ("f", "z3"),
    ]);
    assert!(Layout::new(1, four_two, vec![spread]).is_ok());
    let crowded = zoned(&[
        ("a", "z1"),
        ("b", "z1"),
        ("c", "z1"),
        ("d", "z2"),
        ("e", "z2"),
        ("f", "z3"),
    ]);
    assert_eq!(
        Layout::new(1, four_two, vec![crowded]),
        Err(LayoutError::ZoneOverloaded {
            set: 0,
            zone: "z1".to_owned(),
            shards: 3,
            parity: 2,
        })
    );
}

#[test]
fn several_sets_make_one_layout() {
    let layout = Layout::new(
        2,
        code(2, 1),
        vec![plain(&["a", "b", "c"]), plain(&["d", "e", "f"])],
    )
    .expect("two sets");
    assert_eq!(
        names(&layout),
        vec![vec!["a", "b", "c"], vec!["d", "e", "f"]]
    );
}
