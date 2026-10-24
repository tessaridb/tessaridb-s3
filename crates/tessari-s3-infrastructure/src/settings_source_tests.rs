use std::collections::HashMap;

use super::resolve;
use crate::Error;

fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = vars
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    move |key| map.get(key).cloned()
}

fn held(vars: &[(&str, &str)]) -> HashMap<String, String> {
    vars.iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn the_store_wins_and_what_only_the_environment_holds_is_seeded() {
    let resolved = resolve(
        &env(&[
            ("TESSARIDB_S3_AUDIT", "off"),
            ("TESSARIDB_S3_DOMAINS", "s3.example.com"),
            ("TESSARIDB_S3_LISTEN", "0.0.0.0:9100"),
        ]),
        &held(&[("TESSARIDB_S3_AUDIT", "on")]),
        true,
    )
    .expect("resolves");
    assert_eq!(resolved.value("TESSARIDB_S3_AUDIT"), Some("on"));
    assert_eq!(
        resolved.value("TESSARIDB_S3_DOMAINS"),
        Some("s3.example.com")
    );
    // A machine's own setting is never stored, so it is never resolved here.
    assert_eq!(resolved.value("TESSARIDB_S3_LISTEN"), None);
    assert_eq!(
        resolved.seed(),
        vec![("TESSARIDB_S3_DOMAINS", "s3.example.com")]
    );
    assert_eq!(resolved.differing(), vec!["TESSARIDB_S3_AUDIT"]);
}

#[test]
fn a_fixed_setting_the_environment_contradicts_refuses_the_start_by_name() {
    let refused = resolve(
        &env(&[("TESSARIDB_S3_ERASURE", "4+2")]),
        &held(&[("TESSARIDB_S3_ERASURE", "2+1")]),
        true,
    )
    .expect_err("refused");
    assert!(
        matches!(
            refused,
            Error::InvalidConfig {
                key: "TESSARIDB_S3_ERASURE",
                ..
            }
        ),
        "{refused:?}"
    );
    // The same value is no contradiction.
    let level = resolve(
        &env(&[("TESSARIDB_S3_ERASURE", "2+1")]),
        &held(&[("TESSARIDB_S3_ERASURE", "2+1")]),
        true,
    )
    .expect("resolves");
    assert!(level.differing().is_empty() && level.seed().is_empty());
}

#[test]
fn a_secret_stays_in_the_environment_while_the_vault_cannot_be_opened() {
    let resolved = resolve(
        &env(&[("TESSARIDB_S3_ROOT_SECRET_KEY", "from-the-environment-0123")]),
        &held(&[("TESSARIDB_S3_ROOT_SECRET_KEY", "from-the-store-0123456")]),
        false,
    )
    .expect("resolves");
    assert_eq!(
        resolved.value("TESSARIDB_S3_ROOT_SECRET_KEY"),
        Some("from-the-environment-0123")
    );
    assert!(resolved.seed().is_empty() && resolved.differing().is_empty());
}

#[test]
fn a_blank_environment_value_is_unset_and_neither_seeds_nor_differs() {
    let resolved = resolve(
        &env(&[("TESSARIDB_S3_AUDIT", "  ")]),
        &held(&[("TESSARIDB_S3_AUDIT", "on")]),
        true,
    )
    .expect("resolves");
    assert_eq!(resolved.value("TESSARIDB_S3_AUDIT"), Some("on"));
    assert!(resolved.seed().is_empty() && resolved.differing().is_empty());
}
