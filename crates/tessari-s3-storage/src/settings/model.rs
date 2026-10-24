//! Where a stored setting is kept: its record id.

use tessari_s3_infrastructure::settings_keys::{Reach, STORED, Stored};

/// The variable prefix, which a record id leaves out.
const PREFIX: &str = "TESSARIDB_S3_";

/// The record id of `stored` as `region` reads it: `AUDIT` names a global setting, `us-east-1/AUDIT` a regional one.
#[must_use]
pub(crate) fn record_id(stored: &Stored, region: &str) -> String {
    let short = stored.key.strip_prefix(PREFIX).unwrap_or(stored.key);
    match stored.reach {
        Reach::Global => short.to_owned(),
        Reach::Region => format!("{region}/{short}"),
    }
}

/// One stored setting as an operator sees it; a secret's value is never part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingView {
    /// The variable.
    pub name: &'static str,
    /// Whether one value serves every region.
    pub global: bool,
    /// Whether it is a secret, kept in the vault.
    pub secret: bool,
    /// A plain setting's stored value; `None` for a secret, or when nothing is stored.
    pub value: Option<String>,
    /// Whether the store holds it.
    pub stored: bool,
    /// Whether a running node applies a change to it without a restart.
    pub live: bool,
    /// Whether it may never change once stored.
    pub fixed: bool,
}

/// The variable a record id names for `region`, or `None` for another region's record or one this build does not keep.
#[must_use]
pub(crate) fn variable_of(id: &str, region: &str) -> Option<&'static str> {
    STORED
        .iter()
        .find(|stored| record_id(stored, region) == id)
        .map(|stored| stored.key)
}

#[cfg(test)]
mod tests {
    use super::{record_id, variable_of};
    use tessari_s3_infrastructure::settings_keys::stored_setting;

    #[test]
    fn a_global_setting_is_one_record_and_a_regional_one_a_record_per_region() {
        let iam = stored_setting("TESSARIDB_S3_IAM_KEY").expect("stored");
        let erasure = stored_setting("TESSARIDB_S3_ERASURE").expect("stored");
        assert_eq!(record_id(iam, "eu-west-1"), "IAM_KEY");
        assert_eq!(record_id(erasure, "eu-west-1"), "eu-west-1/ERASURE");
        assert_eq!(
            variable_of("eu-west-1/ERASURE", "eu-west-1"),
            Some("TESSARIDB_S3_ERASURE")
        );
        // Another region's record is not this region's setting.
        assert_eq!(variable_of("us-east-1/ERASURE", "eu-west-1"), None);
        assert_eq!(
            variable_of("IAM_KEY", "eu-west-1"),
            Some("TESSARIDB_S3_IAM_KEY")
        );
    }
}
