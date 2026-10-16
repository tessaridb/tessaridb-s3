//! Block Public Access (S3 ref 04 M5): four switches a bucket carries, and whether a policy is public — public until
//! proven otherwise.

use super::document::{BucketPolicy, Effect, Principals};

/// A bucket's four Block Public Access switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicAccessBlock {
    /// Refuse a write that carries a public ACL (ACLs are never honoured here, so every one is refused anyway).
    pub block_public_acls: bool,
    /// Ignore public ACLs when deciding (none is ever honoured here).
    pub ignore_public_acls: bool,
    /// Refuse to store a public bucket policy.
    pub block_public_policy: bool,
    /// Keep a public policy's Allows to the principals of the bucket's own space.
    pub restrict_public_buckets: bool,
}

impl PublicAccessBlock {
    /// Every switch off: what a removed configuration blocks.
    pub const NONE: Self = Self {
        block_public_acls: false,
        ignore_public_acls: false,
        block_public_policy: false,
        restrict_public_buckets: false,
    };
}

/// Every switch on: a bucket that stored no configuration (the default S3 gives every new bucket).
impl Default for PublicAccessBlock {
    fn default() -> Self {
        Self {
            block_public_acls: true,
            ignore_public_acls: true,
            block_public_policy: true,
            restrict_public_buckets: true,
        }
    }
}

impl BucketPolicy {
    /// Whether any Allow reaches callers it does not name.
    #[must_use]
    pub fn is_public(&self) -> bool {
        self.statements.iter().any(|statement| {
            statement.effect == Effect::Allow
                && statement.principals == Principals::Anyone
                && !statement.conditions.iter().any(|c| c.fixes_who_asks())
        })
    }
}
