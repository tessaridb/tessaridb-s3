use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tessari_s3_constants::LAYOUT_PROPOSE_ATTEMPTS;
use tessari_s3_core::erasure::{Code, Layout};
use tessari_s3_infrastructure::tessaridb::{MetaError, RefusalClass};

use super::{Layouts, current_of, version_of};
use crate::Result;
use crate::cluster::entity::{LayoutEntity, MemberEntity, NodeEntity};
use crate::cluster::repository::{ClusterRepository, Proposed};

/// What a fake store does when a first layout is proposed.
#[derive(Clone, Copy)]
enum OnPropose {
    /// The proposal commits.
    Commits,
    /// Another node's proposal committed first (`winner`).
    Loses,
    /// The first n proposals are refused `retry`; after that, another node's proposal has committed.
    Contended(u32),
}

/// A metadata store in memory: registrations, layouts by version, the current version, and counters.
struct Fake {
    nodes: Vec<(&'static str, Option<&'static str>)>,
    layouts: Mutex<BTreeMap<i64, LayoutEntity>>,
    current: Mutex<Option<i64>>,
    on_propose: OnPropose,
    winner: LayoutEntity,
    proposals: Mutex<u32>,
    layout_reads: Mutex<u32>,
}

impl Fake {
    fn new(nodes: Vec<(&'static str, Option<&'static str>)>, on_propose: OnPropose) -> Self {
        Self {
            nodes,
            layouts: Mutex::new(BTreeMap::new()),
            current: Mutex::new(None),
            on_propose,
            winner: entity(1, &[&["n1", "n2", "n3"]]),
            proposals: Mutex::new(0),
            layout_reads: Mutex::new(0),
        }
    }

    fn commit(&self, layout: &LayoutEntity) {
        self.layouts
            .lock()
            .expect("unpoisoned")
            .insert(layout.version, layout.clone());
        *self.current.lock().expect("unpoisoned") = Some(layout.version);
    }
}

fn entity(version: i64, sets: &[&[&str]]) -> LayoutEntity {
    LayoutEntity {
        version,
        data: 2,
        parity: 1,
        sets: sets
            .iter()
            .map(|set| {
                set.iter()
                    .map(|node| MemberEntity {
                        node: (*node).to_owned(),
                        zone: None,
                    })
                    .collect()
            })
            .collect(),
    }
}

impl ClusterRepository for Fake {
    async fn register(&self, _node: &str, _endpoint: &str, _zone: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn nodes(&self) -> Result<Vec<NodeEntity>> {
        Ok(self
            .nodes
            .iter()
            .map(|(node, zone)| NodeEntity {
                node: (*node).to_owned(),
                endpoint: "x:1".to_owned(),
                zone: zone.map(str::to_owned),
            })
            .collect())
    }

    async fn current(&self) -> Result<Option<i64>> {
        Ok(*self.current.lock().expect("unpoisoned"))
    }

    async fn layout(&self, version: i64) -> Result<Option<LayoutEntity>> {
        let mut reads = self.layout_reads.lock().expect("unpoisoned");
        *reads = reads.saturating_add(1);
        Ok(self
            .layouts
            .lock()
            .expect("unpoisoned")
            .get(&version)
            .cloned())
    }

    async fn propose(&self, layout: &LayoutEntity) -> Result<Proposed> {
        let attempt = {
            let mut proposals = self.proposals.lock().expect("unpoisoned");
            *proposals = proposals.saturating_add(1);
            *proposals
        };
        match self.on_propose {
            OnPropose::Commits => {
                self.commit(layout);
                Ok(Proposed::Created)
            }
            OnPropose::Loses => {
                self.commit(&self.winner);
                Ok(Proposed::Exists)
            }
            OnPropose::Contended(refusals) if attempt <= refusals => {
                Err(crate::Error::Meta(MetaError::Refused {
                    message: "write conflict on record 1".to_owned(),
                    class: Some(RefusalClass::Retry),
                }))
            }
            OnPropose::Contended(_) => {
                self.commit(&self.winner);
                Ok(Proposed::Exists)
            }
        }
    }
}

fn code() -> Code {
    Code::new(2, 1).expect("a code")
}

fn names(layout: &Layout) -> Vec<Vec<(String, Option<String>)>> {
    layout
        .sets()
        .iter()
        .map(|set| {
            set.iter()
                .map(|member| {
                    (
                        member.node().as_str().to_owned(),
                        member.zone().map(|zone| zone.as_str().to_owned()),
                    )
                })
                .collect()
        })
        .collect()
}

fn plain(sets: &[&[&str]]) -> Vec<Vec<(String, Option<String>)>> {
    sets.iter()
        .map(|set| set.iter().map(|node| ((*node).to_owned(), None)).collect())
        .collect()
}

fn cache() -> Layouts {
    Arc::default()
}

#[tokio::test]
async fn a_proposal_that_lost_the_race_takes_the_winners_layout() {
    // The winner saw a node this one has not seen yet, so the two proposals differ.
    let store = Fake::new(
        vec![("n2", None), ("n3", None), ("n4", None)],
        OnPropose::Loses,
    );
    let layout = current_of(&store, &cache(), code())
        .await
        .expect("asked")
        .expect("a layout");
    assert_eq!(
        names(&layout),
        plain(&[&["n1", "n2", "n3"]]),
        "the committed layout, not this node's proposal"
    );
}

#[tokio::test]
async fn a_proposal_refused_retry_is_proposed_again_and_takes_the_winners_layout() {
    let store = Fake::new(
        vec![("n1", None), ("n2", None), ("n3", None)],
        OnPropose::Contended(1),
    );
    let layout = current_of(&store, &cache(), code())
        .await
        .expect("a contended proposal is retried, not reported")
        .expect("a layout");
    assert_eq!(layout.version(), 1);
    assert_eq!(*store.proposals.lock().expect("unpoisoned"), 2);
}

#[tokio::test]
async fn proposals_refused_retry_every_time_give_up_after_the_bound() {
    let store = Fake::new(
        vec![("n1", None), ("n2", None), ("n3", None)],
        OnPropose::Contended(u32::MAX),
    );
    let outcome = current_of(&store, &cache(), code()).await;
    assert!(
        matches!(&outcome, Err(crate::Error::Meta(error)) if error.is_class(RefusalClass::Retry)),
        "{outcome:?}"
    );
    assert_eq!(
        *store.proposals.lock().expect("unpoisoned"),
        LAYOUT_PROPOSE_ATTEMPTS
    );
}

#[tokio::test]
async fn the_first_layout_follows_the_registered_zones() {
    // 2+1 survives one shard per zone: n2 shares n1's zone, so n3 takes its place.
    let store = Fake::new(
        vec![
            ("n1", Some("a")),
            ("n2", Some("a")),
            ("n3", Some("b")),
            ("n4", Some("c")),
        ],
        OnPropose::Commits,
    );
    let layout = current_of(&store, &cache(), code())
        .await
        .expect("asked")
        .expect("a layout");
    let zoned = |node: &str, zone: &str| (node.to_owned(), Some(zone.to_owned()));
    assert_eq!(
        names(&layout),
        vec![vec![zoned("n1", "a"), zoned("n3", "b"), zoned("n4", "c")]]
    );
    let stored = store.layouts.lock().expect("unpoisoned")[&1].clone();
    assert_eq!(
        stored.sets[0][1],
        MemberEntity {
            node: "n3".to_owned(),
            zone: Some("b".to_owned())
        },
        "the zones are stored with the layout"
    );
}

#[tokio::test]
async fn the_current_version_names_the_layout_and_each_version_is_read_once() {
    let store = Fake::new(vec![], OnPropose::Commits);
    store.commit(&entity(1, &[&["n1", "n2", "n3"]]));
    store.commit(&entity(2, &[&["n4", "n5", "n6"], &["n7", "n8", "n9"]]));
    let layouts = cache();
    for _ in 0..3 {
        let layout = current_of(&store, &layouts, code())
            .await
            .expect("asked")
            .expect("a layout");
        assert_eq!(
            (layout.version(), names(&layout)),
            (2, plain(&[&["n4", "n5", "n6"], &["n7", "n8", "n9"]]))
        );
    }
    let first = version_of(&store, &layouts, code(), 1)
        .await
        .expect("version 1 exists");
    assert_eq!(names(&first), plain(&[&["n1", "n2", "n3"]]));
    assert_eq!(
        *store.layout_reads.lock().expect("unpoisoned"),
        2,
        "one read per version, then the cache"
    );
    assert!(
        matches!(
            version_of(&store, &layouts, Code::new(3, 1).expect("a code"), 1).await,
            Err(crate::Error::LayoutMismatch { .. })
        ),
        "a cached layout is held to the asking node's code as a stored one is"
    );
    assert!(matches!(
        version_of(&store, &layouts, code(), 3).await,
        Err(crate::Error::Malformed {
            record: "layout",
            ..
        })
    ));
}
