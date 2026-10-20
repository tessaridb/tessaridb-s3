//! What bucket persistence must answer, whatever stores it.

use std::future::Future;

use tessari_s3_core::authz::{SpaceName, Visible};
use tessari_s3_infrastructure::tessaridb::Value;
use tessari_s3_types::BucketName;

use super::entity::BucketEntity;
use crate::Result;

/// The outcome of inserting a bucket record.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Inserted {
    /// The record was written.
    Created(Box<BucketEntity>),
    /// A record of that name already exists.
    Exists,
}

/// Bucket queries; no rule beyond what a query states.
pub(crate) trait BucketRepository: Send + Sync {
    /// Writes a new bucket record with a fresh incarnation, owned by `space` and made by `creator`; with `object_lock`
    /// it is created with Object Lock on and versioning enabled, in the same write.
    fn insert(
        &self,
        name: &BucketName,
        region: &str,
        space: &SpaceName,
        creator: Option<&str>,
        object_lock: bool,
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

    /// Sets `name`'s versioning state.
    fn set_versioning(
        &self,
        name: &BucketName,
        state: &'static str,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Sets `name`'s CORS rules, or removes them when `rules` is `None`.
    fn set_cors(
        &self,
        name: &BucketName,
        rules: Option<Value>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Sets `name`'s lifecycle rules, or removes them when `rules` is `None`.
    fn set_lifecycle(
        &self,
        name: &BucketName,
        rules: Option<Value>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Sets `name`'s Object Lock configuration while its versioning is enabled; `false` when it was not.
    fn set_object_lock(
        &self,
        name: &BucketName,
        configuration: Value,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Sets `name`'s default encryption, or removes it when `encryption` is `None`.
    fn set_encryption(
        &self,
        name: &BucketName,
        encryption: Option<Value>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Sets `name`'s policy text, or removes it when `policy` is `None`.
    fn set_policy(
        &self,
        name: &BucketName,
        policy: Option<String>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Sets `name`'s Block Public Access text (empty: removed).
    fn set_public_access(
        &self,
        name: &BucketName,
        text: String,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Replaces `name`'s tags, or removes them when `tags` is `None`.
    fn set_tags(
        &self,
        name: &BucketName,
        tags: Option<Value>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Sets `name`'s replication configuration, or removes it when `configuration` is `None`.
    fn set_replication(
        &self,
        name: &BucketName,
        configuration: Option<Value>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Sets `name`'s notification configuration, or removes it when `configuration` is `None`.
    fn set_notification(
        &self,
        name: &BucketName,
        configuration: Option<Value>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Sets `name`'s media rules, or removes them when `rules` is `None`.
    /// Sets `name`'s media rules and counts the change; with `expected`, only while the count is still that, else
    /// `false` and nothing written.
    fn set_media(
        &self,
        name: &BucketName,
        rules: Option<Value>,
        expected: Option<u64>,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Sets `name`'s acknowledgement to its region's majority (`local`) or back to the default.
    fn set_acknowledge(
        &self,
        name: &BucketName,
        local: bool,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Writes one record to this region's database acknowledged by the region's majority, answering the store's own
    /// words when it refuses that acknowledgement.
    fn probe_local(&self) -> impl Future<Output = Result<Option<String>>> + Send;

    fn list(&self, visible: &Visible) -> impl Future<Output = Result<Vec<BucketEntity>>> + Send;
    /// Whether any object record lies in `name`'s range.
    fn holds_objects(&self, name: &BucketName) -> impl Future<Output = Result<bool>> + Send;
    /// Removes `name`'s record, answering what it held, or `None` when there was none.
    fn remove(
        &self,
        name: &BucketName,
    ) -> impl Future<Output = Result<Option<BucketEntity>>> + Send;
}
