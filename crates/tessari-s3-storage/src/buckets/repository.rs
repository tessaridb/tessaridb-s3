//! What bucket persistence must answer, whatever stores it.

use std::future::Future;

use tessari_s3_core::authz::{SpaceName, Visible};
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
    /// Writes a new bucket record with a fresh incarnation, owned by `space` and made by `creator`.
    fn insert(
        &self,
        name: &BucketName,
        region: &str,
        space: &SpaceName,
        creator: Option<&str>,
    ) -> impl Future<Output = Result<Inserted>> + Send;
    /// The record of `name`, if any.
    fn get(&self, name: &BucketName) -> impl Future<Output = Result<Option<BucketEntity>>> + Send;
    /// Every bucket record `visible` covers, filtered in the query.
    /// Sets the stored limits of `name`, an absent one removing that limit; the caller has checked it exists.
    fn set_limits(
        &self,
        name: &BucketName,
        max_bytes: Option<i64>,
        max_objects: Option<i64>,
    ) -> impl Future<Output = Result<()>> + Send;

    fn list(&self, visible: &Visible) -> impl Future<Output = Result<Vec<BucketEntity>>> + Send;
    /// Whether any object record lies in `name`'s range.
    fn holds_objects(&self, name: &BucketName) -> impl Future<Output = Result<bool>> + Send;
    /// Removes `name`'s record, answering what it held, or `None` when there was none.
    fn remove(
        &self,
        name: &BucketName,
    ) -> impl Future<Output = Result<Option<BucketEntity>>> + Send;
}
