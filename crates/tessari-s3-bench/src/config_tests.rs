use std::collections::HashMap;

use super::BenchConfig;
use crate::body::SizeClass;
use crate::error::Error;

fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |key| map.get(key).cloned()
}

const REQUIRED: [(&str, &str); 4] = [
    ("TESSARIDB_S3_BENCH_ENDPOINT", "127.0.0.1:47920"),
    ("TESSARIDB_S3_BENCH_ACCESS_KEY", "bench-access"),
    ("TESSARIDB_S3_BENCH_SECRET_KEY", "bench-secret-0123456789"),
    ("TESSARIDB_S3_BENCH_SERVER_BUILD", "dev@abc1234"),
];

#[test]
fn the_defaults_cover_every_class_and_four_concurrency_steps() {
    let config = BenchConfig::from_lookup(lookup(&REQUIRED)).expect("required variables suffice");
    assert_eq!(config.plan.classes.len(), 5);
    assert_eq!(config.plan.concurrency, vec![1, 4, 16, 64]);
    assert_eq!(config.region, "us-east-1");
    assert_eq!(config.secret_key.expose(), "bench-secret-0123456789");
}

#[test]
fn each_required_variable_is_named_when_missing() {
    for (missing, _) in REQUIRED {
        let rest: Vec<(&str, &str)> = REQUIRED
            .iter()
            .copied()
            .filter(|(k, _)| *k != missing)
            .collect();
        match BenchConfig::from_lookup(lookup(&rest)) {
            Err(Error::MissingConfig { key }) => assert_eq!(key, missing),
            other => panic!("{missing}: {other:?}"),
        }
    }
}

#[test]
fn a_bad_list_is_refused_without_echoing_it() {
    let mut pairs = REQUIRED.to_vec();
    pairs.push(("TESSARIDB_S3_BENCH_CLASSES", "4k,SECRET-LOOKING-VALUE"));
    let error = BenchConfig::from_lookup(lookup(&pairs))
        .map(|_| ())
        .expect_err("unknown class");
    assert!(
        matches!(
            error,
            Error::InvalidConfig {
                key: "TESSARIDB_S3_BENCH_CLASSES",
                ..
            }
        ),
        "{error:?}"
    );
    assert!(
        !error.to_string().contains("SECRET-LOOKING-VALUE"),
        "{error}"
    );
    let mut pairs = REQUIRED.to_vec();
    pairs.push(("TESSARIDB_S3_BENCH_CONCURRENCY", "1,0"));
    assert!(matches!(
        BenchConfig::from_lookup(lookup(&pairs)).map(|_| ()),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_BENCH_CONCURRENCY",
            ..
        })
    ));
}

#[test]
fn chosen_classes_and_steps_are_kept_in_order() {
    let mut pairs = REQUIRED.to_vec();
    pairs.push(("TESSARIDB_S3_BENCH_CLASSES", "1m, 4k"));
    pairs.push(("TESSARIDB_S3_BENCH_CONCURRENCY", "8,2"));
    let config = BenchConfig::from_lookup(lookup(&pairs)).expect("valid lists");
    assert_eq!(
        config.plan.classes,
        vec![SizeClass::Large, SizeClass::Small]
    );
    assert_eq!(config.plan.concurrency, vec![8, 2]);
}
