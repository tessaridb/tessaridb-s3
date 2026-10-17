//! What the access log must answer, whatever stores it.

use std::future::Future;

use super::model::AccessRecord;
use crate::Result;

/// Access-log appends.
pub(crate) trait AuditRepository: Send + Sync {
    /// Appends `records` in one transaction.
    fn append_all(&self, records: &[AccessRecord]) -> impl Future<Output = Result<()>> + Send;
    /// Appends `record` alone; answers `false` when a record with its request id is already logged.
    fn append_one(&self, record: &AccessRecord) -> impl Future<Output = Result<bool>> + Send;
}
