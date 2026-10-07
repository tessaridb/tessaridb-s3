//! What bucket persistence must answer, whatever stores it.

use std::future::Future;

use tessari_s3_types::BucketName;

use super::entity::BucketEntity;
use crate::Result;

/// The outcome of inserting a bucket record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Inserted {
    /// The record was written.
    Created(BucketEntity),
    /// A record of that name already exists.
    Exists,
}

/// Bucket queries; no rule beyond what a query states.
pub(crate) trait BucketRepository: Send + Sync {
    /// Writes a new bucket record with a fresh incarnation.
    fn insert(
        &self,
        name: &BucketName,
        region: &str,
    ) -> impl Future<Output = Result<Inserted>> + Send;
    /// The record of `name`, if any.
    fn get(&self, name: &BucketName) -> impl Future<Output = Result<Option<BucketEntity>>> + Send;
    /// Every bucket record.
    fn list(&self) -> impl Future<Output = Result<Vec<BucketEntity>>> + Send;
    /// Whether any object record lies in `name`'s range.
    fn holds_objects(&self, name: &BucketName) -> impl Future<Output = Result<bool>> + Send;
    /// Removes `name`'s record, answering what it held, or `None` when there was none.
    fn remove(
        &self,
        name: &BucketName,
    ) -> impl Future<Output = Result<Option<BucketEntity>>> + Send;
}
