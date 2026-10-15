//! What topology persistence must answer, whatever stores it.

use std::future::Future;

use super::entity::{LayoutEntity, NodeEntity, StagedEntity};
use crate::Result;

/// The outcome of creating a layout version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Proposed {
    /// This proposal became the layout.
    Created,
    /// Another node's proposal for the version committed first.
    Exists,
}

/// The outcome of applying a staged layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Applied {
    /// The layout became the current version and the staged record is gone.
    Applied,
    /// The current version is no longer the base, or the staged record is no longer the one named; nothing written.
    Refused,
}

/// Topology queries; no rule beyond what a query states.
pub(crate) trait ClusterRepository: Send + Sync {
    /// Writes or refreshes `node`'s registration, in `zone` when it names one.
    fn register(
        &self,
        node: &str,
        endpoint: &str,
        zone: Option<&str>,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Registered nodes in id order, at most the server's cap.
    fn nodes(&self) -> impl Future<Output = Result<Vec<NodeEntity>>> + Send;
    /// The version of the layout new objects are written under, once there is one.
    fn current(&self) -> impl Future<Output = Result<Option<i64>>> + Send;
    /// Layout `version`, if it exists.
    fn layout(&self, version: i64) -> impl Future<Output = Result<Option<LayoutEntity>>> + Send;
    /// Creates the FIRST layout and points the current version at it, in one transaction, unless either exists.
    fn propose(&self, layout: &LayoutEntity) -> impl Future<Output = Result<Proposed>> + Send;
    /// The staged layout, if any.
    fn staged(&self) -> impl Future<Output = Result<Option<StagedEntity>>> + Send;
    /// Writes `staged`, replacing any staged before.
    fn stage(&self, staged: &StagedEntity) -> impl Future<Output = Result<()>> + Send;
    /// Removes the staged layout, if any.
    fn unstage(&self) -> impl Future<Output = Result<()>> + Send;
    /// In one transaction, while the current version is still `base` and the staged record is still `id`: creates
    /// `layout`, makes it current and removes the staged record.
    fn apply(
        &self,
        base: i64,
        id: &str,
        layout: &LayoutEntity,
    ) -> impl Future<Output = Result<Applied>> + Send;
}
