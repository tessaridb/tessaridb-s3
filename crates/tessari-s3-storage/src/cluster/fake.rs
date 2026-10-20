//! A metadata store in memory for the cluster domain's tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tessari_s3_core::erasure::{Code, Layout};
use tessari_s3_infrastructure::tessaridb::{MetaError, RefusalClass};

use super::layouts::Layouts;
use crate::Result;
use crate::cluster::entity::{LayoutEntity, MemberEntity, NodeEntity, StagedEntity};
use crate::cluster::repository::{Applied, ClusterRepository, Proposed};

/// What a fake store does when a first layout is proposed.
#[derive(Clone, Copy)]
pub(super) enum OnPropose {
    /// The proposal commits.
    Commits,
    /// Another node's proposal committed first (`winner`).
    Loses,
    /// The first n proposals are refused `retry`; after that, another node's proposal has committed.
    Contended(u32),
    /// Another node's proposal committed at the store's creation and is seen only after this long: until then every
    /// proposal is refused `retry`, as a commit is before its group is synced.
    ContendedFor(std::time::Duration),
}

/// A metadata store in memory: registrations, layouts by version, the current version, and counters.
pub(super) struct Fake {
    pub(super) nodes: Vec<(&'static str, Option<&'static str>)>,
    pub(super) layouts: Mutex<BTreeMap<i64, LayoutEntity>>,
    pub(super) current: Mutex<Option<i64>>,
    pub(super) on_propose: OnPropose,
    pub(super) winner: LayoutEntity,
    pub(super) proposals: Mutex<u32>,
    pub(super) layout_reads: Mutex<u32>,
    pub(super) staged: Mutex<Option<StagedEntity>>,
    /// Applies refused `retry` before the store answers.
    pub(super) apply_refusals: Mutex<u32>,
    /// Staged by another operator just before the next apply reaches the store.
    pub(super) staged_before_apply: Mutex<Option<StagedEntity>>,
    pub(super) created: tokio::time::Instant,
}

impl Fake {
    pub(super) fn new(
        nodes: Vec<(&'static str, Option<&'static str>)>,
        on_propose: OnPropose,
    ) -> Self {
        Self {
            nodes,
            layouts: Mutex::new(BTreeMap::new()),
            current: Mutex::new(None),
            on_propose,
            winner: entity(1, &[&["n1", "n2", "n3"]]),
            proposals: Mutex::new(0),
            layout_reads: Mutex::new(0),
            staged: Mutex::new(None),
            apply_refusals: Mutex::new(0),
            staged_before_apply: Mutex::new(None),
            created: tokio::time::Instant::now(),
        }
    }

    pub(super) fn commit(&self, layout: &LayoutEntity) {
        self.layouts
            .lock()
            .expect("unpoisoned")
            .insert(layout.version, layout.clone());
        *self.current.lock().expect("unpoisoned") = Some(layout.version);
    }
}

pub(super) fn entity(version: i64, sets: &[&[&str]]) -> LayoutEntity {
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
    async fn register(
        &self,
        _node: &str,
        _fingerprint: &str,
        _endpoint: &str,
        _zone: Option<&str>,
    ) -> Result<bool> {
        Ok(true)
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
            OnPropose::ContendedFor(lag) if self.created.elapsed() < lag => {
                Err(crate::Error::Meta(MetaError::Refused {
                    message: "write conflict on record 1".to_owned(),
                    class: Some(RefusalClass::Retry),
                }))
            }
            OnPropose::ContendedFor(_) => {
                self.commit(&self.winner);
                Ok(Proposed::Exists)
            }
        }
    }

    async fn staged(&self) -> Result<Option<StagedEntity>> {
        Ok(self.staged.lock().expect("unpoisoned").clone())
    }

    async fn stage(&self, staged: &StagedEntity) -> Result<()> {
        *self.staged.lock().expect("unpoisoned") = Some(staged.clone());
        Ok(())
    }

    async fn unstage(&self) -> Result<()> {
        *self.staged.lock().expect("unpoisoned") = None;
        Ok(())
    }

    async fn apply(&self, base: i64, id: &str, layout: &LayoutEntity) -> Result<Applied> {
        {
            let mut refusals = self.apply_refusals.lock().expect("unpoisoned");
            if *refusals > 0 {
                *refusals = refusals.saturating_sub(1);
                return Err(crate::Error::Meta(MetaError::Refused {
                    message: "write conflict on record 1".to_owned(),
                    class: Some(RefusalClass::Retry),
                }));
            }
        }
        let mut staged = self.staged.lock().expect("unpoisoned");
        if let Some(other) = self.staged_before_apply.lock().expect("unpoisoned").take() {
            *staged = Some(other);
        }
        let current = *self.current.lock().expect("unpoisoned");
        if current != Some(base) || staged.as_ref().map(|staged| staged.id.as_str()) != Some(id) {
            return Ok(Applied::Refused);
        }
        self.commit(layout);
        *staged = None;
        Ok(Applied::Applied)
    }
}

pub(super) fn code() -> Code {
    Code::new(2, 1).expect("a code")
}

pub(super) fn names(layout: &Layout) -> Vec<Vec<(String, Option<String>)>> {
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

pub(super) fn plain(sets: &[&[&str]]) -> Vec<Vec<(String, Option<String>)>> {
    sets.iter()
        .map(|set| set.iter().map(|node| ((*node).to_owned(), None)).collect())
        .collect()
}

pub(super) fn cache() -> Layouts {
    Arc::default()
}
