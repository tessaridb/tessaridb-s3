use std::collections::HashSet;

use super::{Class, FROM_ENVIRONMENT, STORED, stored_setting};

/// Every source file that reads a `TESSARIDB_S3_*` variable on the server.
const READERS: [&str; 18] = [
    include_str!("config.rs"),
    include_str!("bootstrap_config.rs"),
    include_str!("cluster_config.rs"),
    include_str!("console_config.rs"),
    include_str!("iam_config.rs"),
    include_str!("keys_config.rs"),
    include_str!("meta_config.rs"),
    include_str!("notify_config.rs"),
    include_str!("observability_config.rs"),
    include_str!("oidc_config.rs"),
    include_str!("oidc_http.rs"),
    include_str!("webhook.rs"),
    include_str!("tls.rs"),
    include_str!("credential.rs"),
    include_str!("replica_client.rs"),
    include_str!("../../tessari-s3-node/src/main.rs"),
    include_str!("../../tessari-s3-node/src/membership.rs"),
    include_str!("../../tessari-s3-node/src/console.rs"),
];

/// The `"TESSARIDB_S3_…"` literals in `text`.
fn named_in(text: &str) -> HashSet<String> {
    text.split('"')
        .filter(|piece| {
            piece.starts_with("TESSARIDB_S3_")
                && piece
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_variable_the_server_reads_has_exactly_one_home() {
    let read: HashSet<String> = READERS.iter().flat_map(|text| named_in(text)).collect();
    // The control: the scan finds what it must, so an empty set cannot pass for agreement.
    assert!(read.contains("TESSARIDB_S3_LISTEN") && read.contains("TESSARIDB_S3_ERASURE"));
    let stored: Vec<&str> = STORED.iter().map(|stored| stored.key).collect();
    let environment: Vec<&str> = FROM_ENVIRONMENT.iter().map(|(key, _)| *key).collect();
    let classified: Vec<&str> = stored.iter().chain(environment.iter()).copied().collect();
    let unique: HashSet<&str> = classified.iter().copied().collect();
    assert_eq!(
        unique.len(),
        classified.len(),
        "a variable is classified twice"
    );
    let mut unclassified: Vec<&String> = read
        .iter()
        .filter(|key| !unique.contains(key.as_str()))
        .collect();
    unclassified.sort();
    assert!(
        unclassified.is_empty(),
        "read but not classified: {unclassified:?}"
    );
    let mut unread: Vec<&&str> = unique.iter().filter(|key| !read.contains(**key)).collect();
    unread.sort();
    assert!(unread.is_empty(), "classified but read nowhere: {unread:?}");
}

#[test]
fn only_secrets_and_settings_are_stored_and_the_rest_stay_in_the_environment() {
    for stored in &STORED {
        assert!(
            matches!(stored.class, Class::Secret | Class::Setting),
            "{}",
            stored.key
        );
    }
    for (key, class) in &FROM_ENVIRONMENT {
        assert!(
            matches!(class, Class::Node | Class::Bootstrap | Class::Master),
            "{key}"
        );
        assert!(stored_setting(key).is_none(), "{key}");
    }
    assert_eq!(
        stored_setting("TESSARIDB_S3_ERASURE").map(|stored| stored.fixed),
        Some(true)
    );
}
