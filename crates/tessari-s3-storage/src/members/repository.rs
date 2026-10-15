//! What membership persistence must answer, whatever stores it.

use std::future::Future;

use super::entity::MemberEntity;
use crate::Result;

/// The fields an approval or a token issue writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grant<'a> {
    /// The node.
    pub(crate) node: &'a str,
    /// The certificate to pin, if any.
    pub(crate) fingerprint: Option<&'a str>,
    /// A join token's SHA-256 and its lifetime in seconds, if one is issued.
    pub(crate) token: Option<(&'a str, u64)>,
    /// The operator's reason.
    pub(crate) reason: &'a str,
    /// Who.
    pub(crate) by: &'a str,
}

/// Membership queries; no rule beyond what a query states. Records are never deleted, so a record read once exists
/// from then on.
pub(crate) trait MemberRepository: Send + Sync {
    /// The record of `node`, if any.
    fn get(&self, node: &str) -> impl Future<Output = Result<Option<MemberEntity>>> + Send;
    /// Every record in node order, at most the server's cap.
    fn list(&self) -> impl Future<Output = Result<Vec<MemberEntity>>> + Send;
    /// Writes `grant` as an approved record — created when there is none, replaced while it is still approved;
    /// `false` when the record is a tombstone, and then nothing is written.
    fn grant(&self, grant: &Grant<'_>) -> impl Future<Output = Result<bool>> + Send;
    /// While `node` is approved and holds the unexpired token whose SHA-256 is `digest`: binds `fingerprint` and
    /// consumes the token in one statement; `false` otherwise, and nothing is written.
    fn bind(
        &self,
        node: &str,
        digest: &str,
        fingerprint: &str,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// While `node` is approved: makes it a tombstone and deletes its registration in one transaction; `false`
    /// otherwise, and nothing is written.
    fn remove(
        &self,
        node: &str,
        by: &str,
        reason: &str,
    ) -> impl Future<Output = Result<bool>> + Send;
}
