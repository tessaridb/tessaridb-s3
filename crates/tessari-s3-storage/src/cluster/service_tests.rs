use std::sync::Mutex;

use tessari_s3_core::erasure::Code;
use tessari_s3_types::NodeId;

use super::layout_of;
use crate::Result;
use crate::cluster::entity::{LayoutEntity, NodeEntity};
use crate::cluster::repository::{ClusterRepository, Proposed};

/// A repository where another node's proposal commits between this node's read and its own proposal.
struct LosesTheRace {
    nodes: Vec<&'static str>,
    winner: LayoutEntity,
    reads: Mutex<u32>,
}

impl ClusterRepository for LosesTheRace {
    async fn register(&self, _node: &str, _endpoint: &str) -> Result<()> {
        Ok(())
    }

    async fn nodes(&self) -> Result<Vec<NodeEntity>> {
        Ok(self
            .nodes
            .iter()
            .map(|node| NodeEntity {
                node: (*node).to_owned(),
                endpoint: "x:1".to_owned(),
            })
            .collect())
    }

    async fn layout(&self, _version: i64) -> Result<Option<LayoutEntity>> {
        let mut reads = self.reads.lock().expect("unpoisoned");
        *reads = reads.saturating_add(1);
        // The first read sees nothing; by the second, the winner has committed.
        Ok((*reads > 1).then(|| self.winner.clone()))
    }

    async fn propose(&self, _layout: &LayoutEntity) -> Result<Proposed> {
        Ok(Proposed::Exists)
    }
}

#[tokio::test]
async fn a_proposal_that_lost_the_race_takes_the_winners_layout() {
    // The winner saw a node this one has not seen yet, so the two proposals differ.
    let repository = LosesTheRace {
        nodes: vec!["n2", "n3", "n4"],
        winner: LayoutEntity {
            version: 1,
            data: 2,
            parity: 1,
            nodes: vec!["n1".to_owned(), "n2".to_owned(), "n3".to_owned()],
        },
        reads: Mutex::new(0),
    };
    let layout = layout_of(&repository, Code::new(2, 1).expect("a code"))
        .await
        .expect("asked")
        .expect("a layout");
    let names: Vec<&str> = layout.nodes().iter().map(NodeId::as_str).collect();
    assert_eq!(
        names,
        ["n1", "n2", "n3"],
        "the committed layout, not this node's proposal"
    );
}
