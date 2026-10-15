//! A bucket's versioning state. A bucket starts never versioned; once versioning is enabled it moves only between
//! `Enabled` and `Suspended` — there is no value for going back, so the transition cannot be expressed.

/// The state of a bucket that has been versioned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Versioning {
    /// Every write keeps the versions before it.
    Enabled,
    /// Writes replace the key's null version; the versions written while enabled are kept.
    Suspended,
}

impl Versioning {
    /// The S3 `Status` value, also the stored one.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
            Self::Suspended => "Suspended",
        }
    }

    /// The state an S3 `Status` names, exactly as S3 spells it; `None` for anything else.
    #[must_use]
    pub fn parse(status: &str) -> Option<Self> {
        match status {
            "Enabled" => Some(Self::Enabled),
            "Suspended" => Some(Self::Suspended),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Versioning;

    #[test]
    fn only_the_two_spellings_s3_uses_name_a_state() {
        for state in [Versioning::Enabled, Versioning::Suspended] {
            assert_eq!(Versioning::parse(state.as_str()), Some(state));
        }
        for other in ["", "enabled", "Disabled", "Unversioned", " Enabled"] {
            assert_eq!(Versioning::parse(other), None, "{other:?}");
        }
    }
}
