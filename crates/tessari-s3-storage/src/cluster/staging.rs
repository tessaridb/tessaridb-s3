//! A layout change: an operator stages a layout over the current version, and applying it makes it version + 1 by
//! compare-and-set, so two operators — or one and a stale page — cannot both move the cluster. A staged layout
//! carries the zones its nodes were registered in, so it is judged on its own when applied.

use std::collections::BTreeMap;

use tessari_s3_constants::LAYOUT_PROPOSE_ATTEMPTS;
use tessari_s3_core::erasure::{Code, Layout, LayoutError, LayoutNode};
use tessari_s3_infrastructure::tessaridb::RefusalClass;
use tessari_s3_types::{NodeId, Zone};

use super::entity::{LayoutEntity, StagedEntity};
use super::layouts::{Layouts, entity_of, held, version_of};
use super::repository::{Applied, ClusterRepository};
use super::service::members;
use crate::{Error, Result};

/// A layout waiting to be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedLayout {
    /// Names this staging; an apply names it back.
    pub id: String,
    /// The current version it was staged over.
    pub base: u32,
    /// The layout as it would be applied: version `base` + 1.
    pub layout: Layout,
}

/// The outcome of staging a layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Staged, replacing anything staged before.
    Staged(StagedLayout),
    /// There is no layout yet to change.
    NoLayoutYet,
    /// A node the layout names has not registered.
    Unregistered(NodeId),
    /// The layout breaks a rule.
    Invalid(LayoutError),
}

/// The outcome of applying the staged layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Apply {
    /// The layout is the current version now.
    Applied(Layout),
    /// Nothing is staged.
    NothingStaged,
    /// What is staged is not what the apply named: staged again, or discarded and staged again, meanwhile.
    StagedChanged,
    /// The current version moved since the layout was staged; it must be staged again over the new one.
    BaseMoved,
}

/// The staged layout, if any.
pub(super) async fn staged_of(
    repository: &impl ClusterRepository,
    code: Code,
) -> Result<Option<StagedLayout>> {
    let Some(stored) = repository.staged().await? else {
        return Ok(None);
    };
    let base = u32::try_from(stored.base).map_err(|_| malformed("base"))?;
    let next = base.checked_add(1).ok_or(malformed("base"))?;
    let layout = held(
        &LayoutEntity {
            version: i64::from(next),
            data: i64::from(code.data()),
            parity: i64::from(code.parity()),
            sets: stored.sets,
        },
        code,
    )
    .map_err(|_| malformed("sets"))?;
    Ok(Some(StagedLayout {
        id: stored.id,
        base,
        layout,
    }))
}

/// Stages `sets` of registered nodes over the current version, each node with the zone it registered in.
pub(super) async fn stage_of(
    repository: &impl ClusterRepository,
    layouts: &Layouts,
    code: Code,
    sets: Vec<Vec<NodeId>>,
) -> Result<Stage> {
    let Some(base) = repository.current().await? else {
        return Ok(Stage::NoLayoutYet);
    };
    let base = u32::try_from(base).map_err(|_| Error::Malformed {
        record: "layout_current",
        reason: "version",
    })?;
    // The current layout must be one this node may use before anything is staged over it.
    version_of(repository, layouts, code, base).await?;
    let zones: BTreeMap<NodeId, Option<Zone>> = members(repository)
        .await?
        .into_iter()
        .map(|member| (member.node, member.zone))
        .collect();
    let mut placed = Vec::with_capacity(sets.len());
    for set in sets {
        let mut nodes = Vec::with_capacity(set.len());
        for node in set {
            let Some(zone) = zones.get(&node) else {
                return Ok(Stage::Unregistered(node));
            };
            nodes.push(LayoutNode::new(node, zone.clone()));
        }
        placed.push(nodes);
    }
    let next = base.checked_add(1).ok_or(Error::Malformed {
        record: "layout_current",
        reason: "version",
    })?;
    let layout = match Layout::new(next, code, placed) {
        Ok(layout) => layout,
        Err(error) => return Ok(Stage::Invalid(error)),
    };
    let staged = StagedEntity {
        id: uuid::Uuid::new_v4().to_string(),
        base: i64::from(base),
        sets: entity_of(&layout).sets,
    };
    repository.stage(&staged).await?;
    Ok(Stage::Staged(StagedLayout {
        id: staged.id,
        base,
        layout,
    }))
}

/// Applies the staged layout named `id`: version `base` + 1 becomes current only while the current version is
/// still `base` and the staged record is still `id`. Refused `retry`, it is tried again, bounded.
pub(super) async fn apply_of(
    repository: &impl ClusterRepository,
    layouts: &Layouts,
    code: Code,
    id: &str,
) -> Result<Apply> {
    let Some(staged) = staged_of(repository, code).await? else {
        return Ok(Apply::NothingStaged);
    };
    if staged.id != id {
        return Ok(Apply::StagedChanged);
    }
    let entity = entity_of(&staged.layout);
    let mut attempt: u32 = 1;
    let applied = loop {
        match repository.apply(i64::from(staged.base), id, &entity).await {
            Err(Error::Meta(error))
                if error.is_class(RefusalClass::Retry) && attempt < LAYOUT_PROPOSE_ATTEMPTS =>
            {
                tracing::debug!(attempt, error = %error, "layout apply contended; applying again");
                attempt = attempt.saturating_add(1);
            }
            outcome => break outcome?,
        }
    };
    match applied {
        Applied::Applied => {
            let _ = layouts
                .insert_async(staged.layout.version(), staged.layout.clone())
                .await;
            Ok(Apply::Applied(staged.layout))
        }
        // Refused: decide which condition failed from what is there now.
        Applied::Refused => match staged_of(repository, code).await? {
            None => Ok(Apply::NothingStaged),
            Some(now) if now.id != id => Ok(Apply::StagedChanged),
            Some(_) => Ok(Apply::BaseMoved),
        },
    }
}

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "layout_staged",
        reason,
    }
}

#[cfg(test)]
#[path = "staging_tests.rs"]
mod tests;
