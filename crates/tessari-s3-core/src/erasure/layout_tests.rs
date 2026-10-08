use tessari_s3_types::NodeId;

use super::{Layout, LayoutError};
use crate::erasure::Code;

fn nodes(ids: &[&str]) -> Vec<NodeId> {
    ids.iter()
        .map(|id| NodeId::new(id).expect("an id"))
        .collect()
}

fn names(layout: &Layout) -> Vec<&str> {
    layout.nodes().iter().map(NodeId::as_str).collect()
}

fn code() -> Code {
    Code::new(2, 1).expect("a code")
}

#[test]
fn the_first_layout_is_the_lowest_ids_whoever_computes_it() {
    let registered = nodes(&["n4", "n2", "n9", "n1", "n2"]);
    let layout = Layout::first(code(), registered.clone()).expect("enough nodes");
    assert_eq!(
        (layout.version(), names(&layout)),
        (1, vec!["n1", "n2", "n4"])
    );
    let mut reversed = registered;
    reversed.reverse();
    assert_eq!(
        Layout::first(code(), reversed),
        Some(layout),
        "order of discovery does not matter"
    );
}

#[test]
fn too_few_distinct_nodes_make_no_layout() {
    assert_eq!(Layout::first(code(), nodes(&["n1", "n2", "n2"])), None);
}

#[test]
fn a_stored_layout_is_checked_before_it_is_believed() {
    assert!(Layout::new(3, code(), nodes(&["a", "b", "c"])).is_ok());
    assert_eq!(
        Layout::new(0, code(), nodes(&["a", "b", "c"])),
        Err(LayoutError::Version)
    );
    assert_eq!(
        Layout::new(1, code(), nodes(&["a", "b"])),
        Err(LayoutError::Width { nodes: 2, width: 3 })
    );
    assert_eq!(
        Layout::new(1, code(), nodes(&["a", "b", "a"])),
        Err(LayoutError::Duplicate)
    );
}
