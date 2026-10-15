use super::NodeId;
use crate::Error;

#[test]
fn a_node_id_is_a_short_lowercase_name() {
    for id in ["n1", "s3-a", "0", "node-07", &"a".repeat(63)] {
        assert_eq!(
            NodeId::new(id).map(|node| node.as_str().to_owned()),
            Ok(id.to_owned())
        );
    }
}

#[test]
fn anything_else_is_refused() {
    for id in [
        "",
        "N1",
        "-n1",
        "n1-",
        "n_1",
        "n 1",
        "n1/../x",
        "ü",
        &"a".repeat(64),
    ] {
        assert_eq!(NodeId::new(id), Err(Error::InvalidNodeId), "{id:?}");
    }
}

#[test]
fn node_ids_order_by_their_bytes() {
    let mut ids: Vec<NodeId> = ["n10", "n2", "n1"]
        .into_iter()
        .map(|id| NodeId::new(id).expect("an id"))
        .collect();
    ids.sort();
    let names: Vec<&str> = ids.iter().map(NodeId::as_str).collect();
    assert_eq!(names, ["n1", "n10", "n2"]);
}
