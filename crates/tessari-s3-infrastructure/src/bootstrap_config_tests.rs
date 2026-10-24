use std::collections::HashMap;

use super::Bootstrap;
use crate::Error;
use crate::config::tests::CREDENTIALS;

fn read(vars: &[(&str, &str)]) -> crate::Result<Bootstrap> {
    let map: HashMap<String, String> = vars
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    Bootstrap::from_lookup(&|key: &str| map.get(key).cloned())
}

#[test]
fn the_way_to_the_store_comes_from_the_environment_alone() {
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_REGION", "eu-west-1"));
    vars.push((
        "TESSARIDB_S3_KEYS_PASSPHRASE",
        "a passphrase of some length",
    ));
    let bootstrap = read(&vars).expect("reads");
    assert_eq!(bootstrap.region, "eu-west-1");
    assert_eq!(bootstrap.meta.addresses, ["127.0.0.1:9080"]);
    assert_eq!(bootstrap.meta.user, "s3");
    assert_eq!(
        bootstrap.keys_passphrase.as_ref().map(|key| key.expose()),
        Some("a passphrase of some length")
    );
    // Without the root credential: it is a stored setting, so the bootstrap does not need it.
    let without_root: Vec<_> = vars
        .iter()
        .copied()
        .filter(|(key, _)| !key.starts_with("TESSARIDB_S3_ROOT_"))
        .collect();
    assert!(read(&without_root).is_ok());
}

#[test]
fn a_region_that_cannot_name_its_database_is_refused_by_name() {
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_REGION", "EU_West"));
    assert!(matches!(
        read(&vars),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_REGION",
            ..
        })
    ));
    let defaulted = read(&CREDENTIALS).expect("reads");
    assert_eq!(defaulted.region, "us-east-1");
    assert!(defaulted.keys_passphrase.is_none());
}
