//! Failures reaching another node of the cluster.

use tessari_s3_types::ErrorCategory;

/// No answer from a peer, as opposed to an answer refusing the request (which is a status, not an error here).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PeerError {
    /// The peer could not be reached, stopped answering, or did not answer within the call's budget.
    #[error("the peer is unavailable: {reason}")]
    Unavailable {
        /// What went wrong, for the log.
        reason: &'static str,
    },
    /// The client could not be built.
    #[error("peer client configuration: {reason}")]
    Configuration {
        /// What is wrong.
        reason: &'static str,
    },
}

impl PeerError {
    /// The taxonomy category: an unreachable peer is `unavailable`, a client that cannot be built is a defect.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::Unavailable { .. } => ErrorCategory::Unavailable,
            Self::Configuration { .. } => ErrorCategory::Internal,
        }
    }
}
