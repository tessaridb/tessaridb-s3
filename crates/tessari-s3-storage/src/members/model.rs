//! Membership as the rest of the server sees it.

use std::fmt;

use tessari_s3_constants::{JOIN_TOKEN_MAX_SECS, JOIN_TOKEN_MIN_SECS};
use tessari_s3_types::{NodeId, Timestamp};
use zeroize::Zeroizing;

/// Whether `text` is `len` lowercase hexadecimal digits.
fn lowercase_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A certificate's SHA-256, 64 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// `text` when it is 64 lowercase hex digits.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        lowercase_hex(text, 64).then(|| Self(text.to_owned()))
    }

    /// The digits.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How long a join token may wait to be used: one minute to one day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenLifetime(u64);

impl TokenLifetime {
    /// `secs` when it lies within the bounds.
    #[must_use]
    pub fn new(secs: u64) -> Option<Self> {
        (JOIN_TOKEN_MIN_SECS..=JOIN_TOKEN_MAX_SECS)
            .contains(&secs)
            .then_some(Self(secs))
    }

    /// The seconds.
    #[must_use]
    pub const fn secs(self) -> u64 {
        self.0
    }
}

/// A one-time join token: 64 lowercase hex digits, shown to the operator once and never stored; scrubbed on drop
/// and never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct JoinToken(Zeroizing<String>);

impl JoinToken {
    /// `text` when it has a token's shape; anything else was never issued.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        lowercase_hex(text, 64).then(|| Self(Zeroizing::new(text.to_owned())))
    }

    /// The digits, for the one answer that shows them.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for JoinToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JoinToken(<redacted>)")
    }
}

/// A member's state; a removed member is a tombstone that never comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberState {
    /// Admitted.
    Approved,
    /// Removed for good.
    Removed,
}

/// One node's membership record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    /// The node.
    pub node: NodeId,
    /// Admitted or removed.
    pub state: MemberState,
    /// The one certificate the node may present, when pinned or bound by a token.
    pub fingerprint: Option<Fingerprint>,
    /// When the unused join token issued for it lapses; `None` when none waits.
    pub token_expires: Option<Timestamp>,
    /// The operator's reason for the last change.
    pub reason: String,
    /// Who made it.
    pub by: String,
    /// When.
    pub at: Timestamp,
}

/// What approving or issuing a token for a node ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approval {
    /// The node is approved as asked.
    Approved,
    /// The node was removed; it is never admitted again.
    Removed,
}

/// What issuing a join token ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issued {
    /// The token, shown this once, and when it lapses.
    Issued {
        /// The token.
        token: JoinToken,
        /// When it lapses unused.
        expires: Timestamp,
    },
    /// The node was removed; it is never admitted again.
    Removed,
}

/// What offering a join token ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joined {
    /// The token was the node's, unused and unexpired: the presented certificate is now the node's.
    Joined,
    /// No such token for this node, already used, lapsed, or the node is removed.
    Refused,
}

/// What removing a node ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    /// The node is a tombstone now and its registration is gone.
    Removed,
    /// No such member.
    Unknown,
    /// It was removed already.
    AlreadyRemoved,
    /// The current layout still places shards on it; apply a layout without it first.
    InLayout,
}

/// Whether a node presenting a certificate with a given fingerprint is admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Admitted.
    Admitted,
    /// No operator admitted this node.
    Unknown,
    /// The node was removed.
    Removed,
    /// A join token was issued and not yet used; until it is, no certificate is the node's.
    AwaitingToken,
    /// The node is pinned to another certificate.
    OtherCertificate,
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
