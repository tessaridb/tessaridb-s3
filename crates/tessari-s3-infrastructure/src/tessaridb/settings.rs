//! Where the metadata store is and how to sign in to it.

use std::num::NonZeroU8;
use std::time::Duration;

use tessari_s3_types::SecretKey;

/// How many copies the metadata cluster keeps of the namespace. Said out loud on purpose: a namespace that never
/// declared its replication is refused on a cluster rather than quietly given one copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replication {
    /// One copy, declined deliberately — a single metadata node.
    None,
    /// This many copies across the cluster.
    Factor(NonZeroU8),
}

impl Replication {
    /// The clause `DEFINE NAMESPACE` takes.
    #[must_use]
    pub fn clause(self) -> String {
        match self {
            Self::None => "REPLICATION NONE".to_owned(),
            Self::Factor(copies) => format!("REPLICATION FACTOR {copies}"),
        }
    }
}

/// The metadata store's address, credential, tenancy and trust.
#[derive(Debug, Clone)]
pub struct MetaSettings {
    /// `host:port` of each metadata node's wire surface, tried in turn.
    pub addresses: Vec<String>,
    /// The namespace's replication, declared when the schema is applied.
    pub replication: Replication,
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
    /// How long a unit of work waits for the answer once sent; past it the connection is dropped and the outcome is
    /// reported unknown (unavailable), never as a failure.
    pub statement_timeout: Duration,
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
    use std::num::NonZeroU8;

    use super::{Replication, is_safe_name};

    #[test]
    fn replication_is_written_as_the_clause_a_namespace_takes() {
        assert_eq!(Replication::None.clause(), "REPLICATION NONE");
        let three = NonZeroU8::new(3).map(Replication::Factor);
        assert_eq!(
            three.map(Replication::clause).as_deref(),
            Some("REPLICATION FACTOR 3")
        );
    }

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
