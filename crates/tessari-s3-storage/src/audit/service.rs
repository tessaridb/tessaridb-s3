//! Appending to the access log: a batch in one transaction, and — when the batch is refused because some of it was
//! logged by an attempt whose answer was lost — each record on its own, the ones already there counted as written.

use tessari_s3_infrastructure::tessaridb::{MetaError, RefusalClass};

use super::model::AccessRecord;
use super::repository::AuditRepository;
use super::tessaridb::TessariAudit;
use crate::{Error, Result};

/// Access-log operations.
#[derive(Clone)]
pub struct AuditService {
    repository: TessariAudit,
}

impl AuditService {
    /// Access-log operations over `repository`.
    #[must_use]
    pub const fn new(repository: TessariAudit) -> Self {
        Self { repository }
    }

    /// Appends `records` and answers how many are now logged — all of them, unless this fails.
    ///
    /// # Errors
    /// The metadata store's refusal or outage; a caller may retry the same records, which are never doubled.
    pub async fn append(&self, records: &[AccessRecord]) -> Result<usize> {
        match self.repository.append_all(records).await {
            Ok(()) => Ok(records.len()),
            Err(Error::Meta(error)) if is_already_logged(&error) => {
                for record in records {
                    self.repository.append_one(record).await?;
                }
                Ok(records.len())
            }
            Err(error) => Err(error),
        }
    }
}

/// Whether a batch was refused because one of its request ids is already logged.
fn is_already_logged(error: &MetaError) -> bool {
    error.is_class(RefusalClass::Conflict)
}
