//! Which layout an object is written or read under: the current version from `layout_current`, any version from
//! the cache or the store, and the first layout proposed once enough nodes in enough zones have registered.

use std::sync::Arc;

use tessari_s3_constants::LAYOUT_PROPOSE_ATTEMPTS;
use tessari_s3_core::erasure::{Code, Layout, LayoutNode};
use tessari_s3_infrastructure::tessaridb::RefusalClass;
use tessari_s3_types::{NodeId, Zone};

use super::entity::{LayoutEntity, MemberEntity};
use super::repository::{ClusterRepository, Proposed};
use super::service::members;
use crate::{Error, Result};

/// Layouts by version. A version is created once and never rewritten, so an entry never goes stale.
pub(super) type Layouts = Arc<scc::HashMap<u32, Layout>>;

/// The layout the current version names, or the first one proposed when there is no current version yet.
pub(super) async fn current_of(
    repository: &impl ClusterRepository,
    layouts: &Layouts,
    code: Code,
) -> Result<Option<Layout>> {
    match repository.current().await? {
        Some(version) => {
            let version = u32::try_from(version).map_err(|_| Error::Malformed {
                record: "layout_current",
                reason: "version",
            })?;
            version_of(repository, layouts, code, version)
                .await
                .map(Some)
        }
        None => first_of(repository, layouts, code).await,
    }
}

/// Layout `version`, from the cache or the store.
pub(super) async fn version_of(
    repository: &impl ClusterRepository,
    layouts: &Layouts,
    code: Code,
    version: u32,
) -> Result<Layout> {
    if let Some(layout) = layouts
        .read_async(&version, |_, layout| layout.clone())
        .await
    {
        return matching(layout, code);
    }
    let stored = repository
        .layout(i64::from(version))
        .await?
        .ok_or(Error::Malformed {
            record: "layout",
            reason: "a version nothing created",
        })?;
    let layout = held(&stored, code)?;
    // Two readers may both miss and both insert; they insert the same immutable layout.
    let _ = layouts.insert_async(version, layout.clone()).await;
    Ok(layout)
}

/// `layout` when it is under `code`; a node configured with another code is refused rather than reinterpreting it.
fn matching(layout: Layout, code: Code) -> Result<Layout> {
    let stored = layout.code();
    if stored == code {
        return Ok(layout);
    }
    Err(Error::LayoutMismatch {
        stored_data: i64::from(stored.data()),
        stored_parity: i64::from(stored.parity()),
        data: code.data(),
        parity: code.parity(),
    })
}

/// This node's proposal for the first layout, or — when another node's proposal committed first — that one.
async fn first_of(
    repository: &impl ClusterRepository,
    layouts: &Layouts,
    code: Code,
) -> Result<Option<Layout>> {
    let members = members(repository).await?;
    let Some(proposal) = Layout::first(
        code,
        members
            .into_iter()
            .map(|member| LayoutNode::new(member.node, member.zone)),
    ) else {
        return Ok(None);
    };
    match propose(repository, &entity_of(&proposal)).await? {
        Proposed::Created => Ok(Some(proposal)),
        // Another node's proposal committed first; it is the layout, whatever this node computed.
        Proposed::Exists => match repository.current().await? {
            Some(version) => {
                let version = u32::try_from(version).map_err(|_| Error::Malformed {
                    record: "layout_current",
                    reason: "version",
                })?;
                version_of(repository, layouts, code, version)
                    .await
                    .map(Some)
            }
            None => Err(Error::Malformed {
                record: "layout",
                reason: "refused as existing, then not found",
            }),
        },
    }
}

/// Proposes `entity`, again when the store answers `retry`: another node's proposal committed after this one's
/// snapshot, and the next attempt is answered that the layout exists. Bounded, so a store that keeps answering
/// `retry` is reported rather than waited on.
async fn propose(repository: &impl ClusterRepository, entity: &LayoutEntity) -> Result<Proposed> {
    let mut attempt: u32 = 1;
    loop {
        match repository.propose(entity).await {
            Err(Error::Meta(error))
                if error.is_class(RefusalClass::Retry) && attempt < LAYOUT_PROPOSE_ATTEMPTS =>
            {
                tracing::debug!(attempt, error = %error, "layout proposal contended; proposing again");
                attempt = attempt.saturating_add(1);
            }
            outcome => return outcome,
        }
    }
}

/// `layout` as stored.
fn entity_of(layout: &Layout) -> LayoutEntity {
    LayoutEntity {
        version: i64::from(layout.version()),
        data: i64::from(layout.code().data()),
        parity: i64::from(layout.code().parity()),
        sets: layout
            .sets()
            .iter()
            .map(|set| {
                set.iter()
                    .map(|member| MemberEntity {
                        node: member.node().as_str().to_owned(),
                        zone: member.zone().map(|zone| zone.as_str().to_owned()),
                    })
                    .collect()
            })
            .collect(),
    }
}

/// A stored layout as this node may use it: under the code it is configured with, and well formed.
fn held(stored: &LayoutEntity, code: Code) -> Result<Layout> {
    if stored.data != i64::from(code.data()) || stored.parity != i64::from(code.parity()) {
        return Err(Error::LayoutMismatch {
            stored_data: stored.data,
            stored_parity: stored.parity,
            data: code.data(),
            parity: code.parity(),
        });
    }
    let malformed = |reason| Error::Malformed {
        record: "layout",
        reason,
    };
    let version = u32::try_from(stored.version).map_err(|_| malformed("version"))?;
    let sets = stored
        .sets
        .iter()
        .map(|set| {
            set.iter()
                .map(|member| {
                    let node = NodeId::new(&member.node).map_err(|_| malformed("node id"))?;
                    let zone = member
                        .zone
                        .as_deref()
                        .map(|zone| Zone::new(zone).map_err(|_| malformed("zone")))
                        .transpose()?;
                    Ok(LayoutNode::new(node, zone))
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    Layout::new(version, code, sets).map_err(|_| malformed("shape"))
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
