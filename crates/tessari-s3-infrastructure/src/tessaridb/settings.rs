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

/// Whether `region` is a region name this server accepts: lowercase letters, digits and single hyphens, beginning
/// with a letter and ending with a letter or digit — the shape of the names S3 clients sign with. It becomes part of
/// a database name, so it is held to a pattern with no character of its own meaning there.
#[must_use]
pub fn is_region_name(region: &str) -> bool {
    region.starts_with(|c: char| c.is_ascii_lowercase())
        && !region.ends_with('-')
        && !region.contains("--")
        && region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The database holding the object metadata of `region`: `<database>_<region>` with each hyphen written as an
/// underscore, or `None` when `region` is not a region name or the result is not a name a `USE` may carry.
#[must_use]
pub fn regional_database(database: &str, region: &str) -> Option<String> {
    if !is_region_name(region) {
        return None;
    }
    let name = format!("{database}_{}", region.replace('-', "_"));
    is_safe_name(&name).then_some(name)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use super::{Replication, is_region_name, is_safe_name, regional_database};

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

    #[test]
    fn a_region_is_lowercase_words_joined_by_single_hyphens() {
        for region in ["us-east-1", "eu-west-2", "eu", "ap-southeast-10", "home"] {
            assert!(is_region_name(region), "{region}");
        }
        for region in [
            "", "EU", "1eu", "-eu", "eu-", "eu--west", "eu_west", "eu west", "eu.west", "é",
        ] {
            assert!(!is_region_name(region), "{region}");
        }
    }

    #[test]
    fn a_regions_database_is_the_global_one_suffixed_with_the_region() {
        assert_eq!(
            regional_database("meta", "us-east-1").as_deref(),
            Some("meta_us_east_1")
        );
        assert_eq!(regional_database("meta", "eu").as_deref(), Some("meta_eu"));
        assert_eq!(regional_database("meta", "eu_west"), None, "not a region");
        assert_eq!(regional_database("meta", "EU"), None, "not a region");
        let long = "a".repeat(60);
        assert_eq!(
            regional_database(&long, "eu-1"),
            None,
            "longer than a USE takes"
        );
    }
}
