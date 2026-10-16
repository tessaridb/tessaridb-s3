//! A bucket's Block Public Access as stored: absent while never set (every switch on, as S3 gives a new bucket), four
//! `0`/`1` flags once set, and an empty text once removed (every switch off, answered as no configuration).

use tessari_s3_core::authz::policy::PublicAccessBlock;

use crate::{Error, Result};

/// The configuration a bucket carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PublicAccess {
    /// Never set: every switch on.
    #[default]
    Unset,
    /// Set by a caller.
    Set(PublicAccessBlock),
    /// Removed by a caller: nothing is blocked.
    Removed,
}

impl PublicAccess {
    /// The switches in force.
    #[must_use]
    pub fn in_force(self) -> PublicAccessBlock {
        match self {
            Self::Unset => PublicAccessBlock::default(),
            Self::Set(block) => block,
            Self::Removed => PublicAccessBlock::NONE,
        }
    }

    /// The configuration a caller reads back; `None` once removed.
    #[must_use]
    pub fn answered(self) -> Option<PublicAccessBlock> {
        match self {
            Self::Removed => None,
            other => Some(other.in_force()),
        }
    }
}

/// The stored text of `block` (`None` removes it): the four switches in S3's order.
pub(super) fn to_text(block: Option<PublicAccessBlock>) -> String {
    block.map_or_else(String::new, |block| {
        [
            block.block_public_acls,
            block.ignore_public_acls,
            block.block_public_policy,
            block.restrict_public_buckets,
        ]
        .iter()
        .map(|on| if *on { '1' } else { '0' })
        .collect()
    })
}

/// The configuration a stored text describes.
pub(super) fn from_text(text: Option<&str>) -> Result<PublicAccess> {
    let malformed = || Error::Malformed {
        record: "bucket",
        reason: "public_access is not four 0/1 flags",
    };
    let Some(text) = text else {
        return Ok(PublicAccess::Unset);
    };
    if text.is_empty() {
        return Ok(PublicAccess::Removed);
    }
    let flags = text
        .bytes()
        .map(|b| match b {
            b'0' => Ok(false),
            b'1' => Ok(true),
            _ => Err(malformed()),
        })
        .collect::<Result<Vec<_>>>()?;
    let [acls, ignore, policy, restrict] = flags[..] else {
        return Err(malformed());
    };
    Ok(PublicAccess::Set(PublicAccessBlock {
        block_public_acls: acls,
        ignore_public_acls: ignore,
        block_public_policy: policy,
        restrict_public_buckets: restrict,
    }))
}
