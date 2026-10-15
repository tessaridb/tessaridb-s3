//! What space persistence must answer, whatever stores it.

use std::future::Future;

use tessari_s3_core::authz::SpaceName;

use super::entity::SpaceEntity;
use crate::Result;

/// The outcome of inserting a space record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Inserted {
    /// The record was written.
    Created(SpaceEntity),
    /// A space of that name already exists.
    Exists,
}

/// Space queries; no rule beyond what a query states.
pub(crate) trait SpaceRepository: Send + Sync {
    /// Writes a new space record.
    fn insert(&self, name: &SpaceName) -> impl Future<Output = Result<Inserted>> + Send;
    /// The record of `name`, if any.
    fn get(&self, name: &SpaceName) -> impl Future<Output = Result<Option<SpaceEntity>>> + Send;
    /// Every space record.
    fn list(&self) -> impl Future<Output = Result<Vec<SpaceEntity>>> + Send;
}
