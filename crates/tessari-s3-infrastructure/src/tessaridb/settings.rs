//! Where the metadata store is and how to sign in to it.

use tessari_s3_types::SecretKey;

/// The metadata store's address, credential, tenancy and trust.
#[derive(Debug, Clone)]
pub struct MetaSettings {
    /// `host:port` of a node's wire surface.
    pub address: String,
    /// The user to sign in as.
    pub user: String,
    /// That user's password.
    pub password: SecretKey,
    /// The namespace every unit of work selects; a name checked against `[a-z][a-z0-9_]*`.
    pub namespace: String,
    /// The database every unit of work selects; checked like the namespace.
    pub database: String,
    /// PEM certificate authority to verify the node against; `None` speaks in the clear.
    pub trust_pem: Option<Vec<u8>>,
    /// Connections open at once.
    pub max_connections: usize,
}

/// Whether `name` is a tenancy name this server will put into a `USE`: a name is grammar and is never quoted, so it
/// is held to a pattern narrower than the node's lexer.
#[must_use]
pub fn is_safe_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && name.len() <= 63
}

#[cfg(test)]
mod tests {
    use super::is_safe_name;

    #[test]
    fn only_lowercase_identifiers_reach_a_use() {
        for name in ["s3", "meta", "t_0a1b"] {
            assert!(is_safe_name(name), "{name}");
        }
        for name in [
            "",
            "S3",
            "1s3",
            "s3;",
            "s3 meta",
            "s3-meta",
            "`x`",
            &"a".repeat(64),
        ] {
            assert!(!is_safe_name(name), "{name}");
        }
    }
}
