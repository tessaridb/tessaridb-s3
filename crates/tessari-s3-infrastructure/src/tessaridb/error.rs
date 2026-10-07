//! What a unit of work against the metadata store can end in.

use tessari_s3_types::ErrorCategory;
use tessaridb_client::RefusalClass;

/// A metadata-store failure. A refusal and an outage stay apart: one is the node's answer, the other is the absence
/// of one, and only the second is worth retrying elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetaError {
    /// The node refused the statement, in its own words.
    #[error("the metadata store refused: {message}")]
    Refused {
        /// The node's own message.
        message: String,
        /// What to do about it; `None` from a node that sends words only.
        class: Option<RefusalClass>,
    },
    /// No answer: the node could not be reached or the connection broke.
    #[error("the metadata store is unavailable: {reason}")]
    Unavailable {
        /// What went wrong, for the log.
        reason: String,
    },
    /// The answer did not have the shape this server expects.
    #[error("unexpected answer from the metadata store: {reason}")]
    Protocol {
        /// What was wrong with it.
        reason: &'static str,
    },
    /// The pool's configuration cannot work.
    #[error("metadata store configuration: {reason}")]
    Configuration {
        /// What is wrong.
        reason: &'static str,
    },
    /// A thread panicked holding the pool's lock; the pool refuses rather than guessing at its state.
    #[error("the metadata pool's lock is poisoned")]
    Poisoned,
}

impl MetaError {
    /// Whether the node refused with this class.
    #[must_use]
    pub fn is_class(&self, wanted: RefusalClass) -> bool {
        matches!(self, Self::Refused { class: Some(class), .. } if *class == wanted)
    }

    /// The taxonomy category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Refused {
                class: Some(RefusalClass::Conflict | RefusalClass::Retry),
                ..
            } => ErrorCategory::Conflict,
            Self::Unavailable { .. } => ErrorCategory::Unavailable,
            Self::Refused { .. }
            | Self::Protocol { .. }
            | Self::Configuration { .. }
            | Self::Poisoned => ErrorCategory::Internal,
        }
    }
}

/// The result of a metadata-store call.
pub type MetaResult<T> = std::result::Result<T, MetaError>;
