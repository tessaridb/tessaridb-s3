//! What the replicators' group, the version stamps and the region registry must answer, whatever stores them.

use std::future::Future;

use super::model::Progress;
use crate::Result;
use crate::topic_groups::MessageEntity;

/// Replication queries.
pub(crate) trait ReplicationRepository: Send + Sync {
    /// Declares the replicators' group when it is missing; a new group starts at the topic's oldest message.
    fn define(&self) -> impl Future<Output = Result<()>> + Send;
    /// Hands out up to `limit` tasks.
    fn read(&self, limit: usize) -> impl Future<Output = Result<Vec<MessageEntity>>> + Send;
    /// Settles `positions` for good.
    fn ack(&self, positions: &[u64]) -> impl Future<Output = Result<()>> + Send;
    /// Hands `positions` out again no sooner than `delay_secs` from now.
    fn nack(&self, positions: &[u64], delay_secs: u64) -> impl Future<Output = Result<()>> + Send;
    /// Stamps `status` on version `seq` of `bucket/key` while it is still `version_id`, and on the key's current
    /// object while that is the same version; a version or object gone meanwhile is left as it is.
    fn stamp(
        &self,
        bucket: &str,
        key: &str,
        seq: u64,
        version_id: &str,
        status: &'static str,
    ) -> impl Future<Output = Result<()>> + Send;
    /// The replicators' backlog, in flight and given-up counts, as the topic reports them.
    fn progress(&self) -> impl Future<Output = Result<Progress>> + Send;
    /// Records that `region`'s S3 surface is reached at `endpoint`.
    fn advertise(&self, region: &str, endpoint: &str) -> impl Future<Output = Result<()>> + Send;
    /// The URL `region`'s S3 surface is reached at, if any of its nodes advertised one.
    fn endpoint(&self, region: &str) -> impl Future<Output = Result<Option<String>>> + Send;
}
