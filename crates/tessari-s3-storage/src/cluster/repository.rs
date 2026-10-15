//! What topology persistence must answer, whatever stores it.

use std::future::Future;

use super::entity::{LayoutEntity, NodeEntity};
use crate::Result;

/// The outcome of creating a layout version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Proposed {
    /// This proposal became the layout.
    Created,
    /// Another node's proposal for the version committed first.
    Exists,
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
}
