use std::sync::Mutex;

use tessari_s3_constants::LAYOUT_PROPOSE_ATTEMPTS;
use tessari_s3_core::erasure::Code;
use tessari_s3_infrastructure::tessaridb::{MetaError, RefusalClass};
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

/// A repository where proposals overlap: the first `contended` proposals are refused `retry` because another node's
/// proposal committed after their snapshot; from then on the store answers that the layout exists.
struct Overlaps {
    contended: u32,
    proposals: Mutex<u32>,
    winner: LayoutEntity,
}

impl ClusterRepository for Overlaps {
    async fn register(&self, _node: &str, _endpoint: &str) -> Result<()> {
        Ok(())
    }

    async fn nodes(&self) -> Result<Vec<NodeEntity>> {
        Ok(["n1", "n2", "n3"]
            .iter()
            .map(|node| NodeEntity {
                node: (*node).to_owned(),
                endpoint: "x:1".to_owned(),
            })
            .collect())
    }

    async fn layout(&self, _version: i64) -> Result<Option<LayoutEntity>> {
        let proposals = *self.proposals.lock().expect("unpoisoned");
        Ok((proposals > 0).then(|| self.winner.clone()))
    }

    async fn propose(&self, _layout: &LayoutEntity) -> Result<Proposed> {
        let mut proposals = self.proposals.lock().expect("unpoisoned");
        *proposals = proposals.saturating_add(1);
        if *proposals <= self.contended {
            Err(crate::Error::Meta(MetaError::Refused {
                message: "write conflict on record 1".to_owned(),
                class: Some(RefusalClass::Retry),
            }))
        } else {
            Ok(Proposed::Exists)
        }
    }
}

fn overlaps(contended: u32) -> Overlaps {
    Overlaps {
        contended,
        proposals: Mutex::new(0),
        winner: LayoutEntity {
            version: 1,
            data: 2,
            parity: 1,
            nodes: vec!["n1".to_owned(), "n2".to_owned(), "n3".to_owned()],
        },
    }
}

#[tokio::test]
async fn a_proposal_refused_retry_is_proposed_again_and_takes_the_winners_layout() {
    let repository = overlaps(1);
    let layout = layout_of(&repository, Code::new(2, 1).expect("a code"))
        .await
        .expect("a contended proposal is retried, not reported")
        .expect("a layout");
    assert_eq!(layout.version(), 1);
    assert_eq!(*repository.proposals.lock().expect("unpoisoned"), 2);
}

#[tokio::test]
async fn proposals_refused_retry_every_time_give_up_after_the_bound() {
    let repository = overlaps(u32::MAX);
    let outcome = layout_of(&repository, Code::new(2, 1).expect("a code")).await;
    assert!(
        matches!(&outcome, Err(crate::Error::Meta(error)) if error.is_class(RefusalClass::Retry)),
        "{outcome:?}"
    );
    assert_eq!(
        *repository.proposals.lock().expect("unpoisoned"),
        LAYOUT_PROPOSE_ATTEMPTS
    );
}
