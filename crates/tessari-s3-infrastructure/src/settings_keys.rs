//! Where each `TESSARIDB_S3_*` setting lives: the environment, or the metadata store.
//!
//! A node takes from its environment only what describes the machine it runs on, what it needs to reach the metadata
//! store, and the one passphrase that opens the secrets kept there. Every other setting is kept in the metadata
//! store, written there from the environment the first time a node starts with it, and read from there afterwards.

/// What a setting is, which decides where it is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Describes this machine or process; read from the environment only.
    Node,
    /// How to reach the metadata store; read from the environment only.
    Bootstrap,
    /// The passphrase that opens the secrets vault; read from the environment only, never stored.
    Master,
    /// A secret kept in the metadata store's secrets vault.
    Secret,
    /// A setting kept in the metadata store as a plain record.
    Setting,
}

/// Whether a stored setting is one value for every region or one per region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// One value, shared by every region.
    Global,
    /// One value per region.
    Region,
}

/// A setting the metadata store keeps: its variable, its class, its reach, and whether it may change once stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stored {
    /// The variable's full name.
    pub key: &'static str,
    /// [`Class::Secret`] or [`Class::Setting`].
    pub class: Class,
    /// Global or per region.
    pub reach: Reach,
    /// Written once: a node whose environment names a different value refuses to start.
    pub fixed: bool,
}

const fn stored(key: &'static str, class: Class, reach: Reach, fixed: bool) -> Stored {
    Stored {
        key,
        class,
        reach,
        fixed,
    }
}

/// Every setting the metadata store keeps.
pub const STORED: [Stored; 25] = [
    stored(
        "TESSARIDB_S3_ROOT_ACCESS_KEY",
        Class::Setting,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_ROOT_SECRET_KEY",
        Class::Secret,
        Reach::Global,
        false,
    ),
    stored("TESSARIDB_S3_IAM_KEY", Class::Secret, Reach::Global, false),
    stored(
        "TESSARIDB_S3_CONSOLE_OIDC_CLIENT_SECRET",
        Class::Secret,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CONSOLE_OIDC_CLIENT_KEY",
        Class::Secret,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CONSOLE_BREAK_GLASS_WEBHOOK",
        Class::Secret,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE",
        Class::Setting,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE",
        Class::Setting,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CONSOLE_KEY_SIGN_IN",
        Class::Setting,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CONSOLE_OIDC_ISSUER",
        Class::Setting,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CONSOLE_OIDC_CLIENT_ID",
        Class::Setting,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CONSOLE_OIDC_REDIRECT_URI",
        Class::Setting,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CONSOLE_OIDC_MFA_AMR",
        Class::Setting,
        Reach::Global,
        false,
    ),
    stored(
        "TESSARIDB_S3_CLUSTER_SECRET",
        Class::Secret,
        Reach::Region,
        true,
    ),
    stored(
        "TESSARIDB_S3_NOTIFY_TARGETS",
        Class::Secret,
        Reach::Region,
        false,
    ),
    stored("TESSARIDB_S3_ERASURE", Class::Setting, Reach::Region, true),
    stored(
        "TESSARIDB_S3_META_REPLICATION",
        Class::Setting,
        Reach::Region,
        false,
    ),
    stored("TESSARIDB_S3_DOMAINS", Class::Setting, Reach::Region, false),
    stored(
        "TESSARIDB_S3_RECLAIM_GRACE_SECS",
        Class::Setting,
        Reach::Region,
        false,
    ),
    stored(
        "TESSARIDB_S3_RECLAIM_INTERVAL_SECS",
        Class::Setting,
        Reach::Region,
        false,
    ),
    stored(
        "TESSARIDB_S3_UPLOAD_MAX_AGE_SECS",
        Class::Setting,
        Reach::Region,
        false,
    ),
    stored(
        "TESSARIDB_S3_USAGE_INTERVAL_SECS",
        Class::Setting,
        Reach::Region,
        false,
    ),
    stored(
        "TESSARIDB_S3_HEAL_INTERVAL_SECS",
        Class::Setting,
        Reach::Region,
        false,
    ),
    stored(
        "TESSARIDB_S3_MOVE_BYTES_PER_SEC",
        Class::Setting,
        Reach::Region,
        false,
    ),
    stored("TESSARIDB_S3_AUDIT", Class::Setting, Reach::Region, false),
];

/// Every setting the environment keeps, with its class.
pub const FROM_ENVIRONMENT: [(&str, Class); 33] = [
    ("TESSARIDB_S3_LISTEN", Class::Node),
    ("TESSARIDB_S3_TLS_CERT", Class::Node),
    ("TESSARIDB_S3_TLS_KEY", Class::Node),
    ("TESSARIDB_S3_DATA_DIR", Class::Node),
    ("TESSARIDB_S3_DATA_MIN_FREE_BYTES", Class::Node),
    ("TESSARIDB_S3_NODE_ID", Class::Node),
    ("TESSARIDB_S3_ZONE", Class::Node),
    ("TESSARIDB_S3_REGION", Class::Node),
    ("TESSARIDB_S3_ADVERTISE", Class::Node),
    ("TESSARIDB_S3_INTERNAL_LISTEN", Class::Node),
    ("TESSARIDB_S3_INTERNAL_ADVERTISE", Class::Node),
    ("TESSARIDB_S3_INTERNAL_TLS_CA", Class::Node),
    ("TESSARIDB_S3_INTERNAL_TLS_CERT", Class::Node),
    ("TESSARIDB_S3_INTERNAL_TLS_KEY", Class::Node),
    ("TESSARIDB_S3_CONSOLE_LISTEN", Class::Node),
    ("TESSARIDB_S3_CONSOLE_TLS_CERT", Class::Node),
    ("TESSARIDB_S3_CONSOLE_TLS_KEY", Class::Node),
    ("TESSARIDB_S3_METRICS_LISTEN", Class::Node),
    ("TESSARIDB_S3_SHUTDOWN_GRACE_SECS", Class::Node),
    ("TESSARIDB_S3_FFMPEG", Class::Node),
    ("TESSARIDB_S3_MEDIA_WORKERS", Class::Node),
    ("TESSARIDB_S3_MAX_INFLIGHT", Class::Node),
    ("TESSARIDB_S3_JOIN_TOKEN", Class::Node),
    ("TESSARIDB_S3_FOUNDING_MEMBERS", Class::Node),
    ("TESSARIDB_S3_META_ADDRESS", Class::Bootstrap),
    ("TESSARIDB_S3_META_USER", Class::Bootstrap),
    ("TESSARIDB_S3_META_PASSWORD", Class::Bootstrap),
    ("TESSARIDB_S3_META_CA", Class::Bootstrap),
    ("TESSARIDB_S3_META_NAMESPACE", Class::Bootstrap),
    ("TESSARIDB_S3_META_DATABASE", Class::Bootstrap),
    ("TESSARIDB_S3_META_MAX_CONNECTIONS", Class::Bootstrap),
    ("TESSARIDB_S3_META_STATEMENT_TIMEOUT_SECS", Class::Bootstrap),
    ("TESSARIDB_S3_KEYS_PASSPHRASE", Class::Master),
];

/// The stored setting named `key`, if the metadata store keeps it.
#[must_use]
pub fn stored_setting(key: &str) -> Option<&'static Stored> {
    STORED.iter().find(|stored| stored.key == key)
}

#[cfg(test)]
#[path = "settings_keys_tests.rs"]
mod tests;
