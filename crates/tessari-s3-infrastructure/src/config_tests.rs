use std::collections::HashMap;

use super::S3Config;
use crate::Error;
use crate::tessaridb::Replication;

pub(crate) fn load(vars: &[(&str, &str)]) -> crate::Result<S3Config> {
    let map: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    S3Config::from_lookup(|key| map.get(key).cloned())
}

pub(crate) const CREDENTIALS: [(&str, &str); 6] = [
    ("TESSARIDB_S3_ROOT_ACCESS_KEY", "AKIAIOSFODNN7EXAMPLE"),
    (
        "TESSARIDB_S3_ROOT_SECRET_KEY",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    ),
    ("TESSARIDB_S3_META_ADDRESS", "127.0.0.1:9080"),
    ("TESSARIDB_S3_META_USER", "s3"),
    ("TESSARIDB_S3_META_PASSWORD", "meta-password-0123"),
    ("TESSARIDB_S3_DATA_DIR", "/srv/s3"),
];

#[test]
fn defaults_apply_when_only_the_credentials_are_set() {
    let config = load(&CREDENTIALS).expect("loads");
    assert_eq!(config.listen.to_string(), "127.0.0.1:9100");
    assert_eq!(config.region, "us-east-1");
    assert!(config.domains.is_empty());
    assert_eq!(
        (config.max_inflight, config.shutdown_grace_secs),
        (1024, 30)
    );
    assert_eq!(
        (
            config.meta.namespace.as_str(),
            config.meta.database.as_str()
        ),
        ("s3", "meta")
    );
    assert_eq!(config.meta.max_connections, 32);
    assert!(config.meta.trust_pem.is_none());
}

#[test]
fn a_tenancy_name_that_is_not_an_identifier_is_refused() {
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_META_NAMESPACE", "s3; DROP"));
    let refusal = load(&vars).map(|_| ()).expect_err("refused");
    assert!(
        matches!(
            refusal,
            Error::InvalidConfig {
                key: "TESSARIDB_S3_META_NAMESPACE",
                ..
            }
        ),
        "{refusal:?}"
    );
}

#[test]
fn the_data_directory_is_required_and_taken_as_given() {
    // Every object byte lives on the drives (ADR-0008), so a node with nowhere to put them does not start.
    let without: Vec<_> = CREDENTIALS
        .iter()
        .copied()
        .filter(|(key, _)| *key != "TESSARIDB_S3_DATA_DIR")
        .collect();
    let refusal = load(&without).expect_err("refused");
    assert!(
        matches!(
            refusal,
            Error::MissingConfig {
                key: "TESSARIDB_S3_DATA_DIR"
            }
        ),
        "{refusal:?}"
    );
    let mut vars = without;
    vars.push(("TESSARIDB_S3_DATA_DIR", " /srv/s3 "));
    assert_eq!(
        load(&vars).expect("loads").data_dir,
        std::path::PathBuf::from("/srv/s3")
    );
}

#[test]
fn domains_are_split_trimmed_and_lowercased() {
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_DOMAINS", " S3.Example.com, ,s3.local"));
    assert_eq!(
        load(&vars).expect("loads").domains,
        ["s3.example.com", "s3.local"]
    );
}

#[test]
fn missing_credentials_are_refused_by_name() {
    assert_eq!(
        load(&[]).map(|_| ()),
        Err(Error::MissingConfig {
            key: "TESSARIDB_S3_ROOT_ACCESS_KEY"
        })
    );
    let only_key = [CREDENTIALS[0]];
    assert_eq!(
        load(&only_key).map(|_| ()),
        Err(Error::MissingConfig {
            key: "TESSARIDB_S3_ROOT_SECRET_KEY"
        })
    );
}

#[test]
fn invalid_values_are_refused_without_echoing_them() {
    let cases = [
        ("TESSARIDB_S3_ROOT_SECRET_KEY", "tiny-k3y"),
        ("TESSARIDB_S3_MAX_INFLIGHT", "0"),
        ("TESSARIDB_S3_LISTEN", "nowhere"),
        ("TESSARIDB_S3_SHUTDOWN_GRACE_SECS", "-1"),
    ];
    for (key, value) in cases {
        let mut vars = CREDENTIALS.to_vec();
        vars.retain(|(k, _)| *k != key);
        vars.push((key, value));
        let refusal = load(&vars).map(|_| ()).expect_err(key);
        assert!(
            matches!(refusal, Error::InvalidConfig { key: k, .. } if k == key),
            "{refusal:?}"
        );
        assert!(
            !refusal.to_string().contains(value),
            "the value is not echoed: {refusal}"
        );
    }
}

#[test]
fn the_secret_never_appears_in_debug_output() {
    let config = load(&CREDENTIALS).expect("loads");
    let printed = format!("{config:?}");
    assert!(
        !printed.contains("EXAMPLEKEY") && !printed.contains("meta-password"),
        "{printed}"
    );
}

#[test]
fn reclamation_defaults_and_overrides() {
    let config = load(&CREDENTIALS).expect("loads");
    assert_eq!(
        (config.reclaim_grace_secs, config.reclaim_interval_secs),
        (86_400, 300)
    );
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_RECLAIM_GRACE_SECS", "3600"));
    vars.push(("TESSARIDB_S3_RECLAIM_INTERVAL_SECS", "60"));
    let config = load(&vars).expect("loads");
    assert_eq!(
        (config.reclaim_grace_secs, config.reclaim_interval_secs),
        (3600, 60)
    );
    let mut zero = CREDENTIALS.to_vec();
    zero.push(("TESSARIDB_S3_RECLAIM_INTERVAL_SECS", "0"));
    assert!(matches!(
        load(&zero).map(|_| ()),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_RECLAIM_INTERVAL_SECS",
            ..
        })
    ));
}

#[test]
fn an_upload_lives_a_week_unless_configured_and_never_zero_seconds() {
    let config = load(&CREDENTIALS).expect("loads");
    assert_eq!(config.upload_max_age_secs, 7 * 24 * 60 * 60);
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_UPLOAD_MAX_AGE_SECS", "3600"));
    assert_eq!(load(&vars).expect("loads").upload_max_age_secs, 3600);
    for bad in ["0", "-1", "soon"] {
        let mut refused = CREDENTIALS.to_vec();
        refused.push(("TESSARIDB_S3_UPLOAD_MAX_AGE_SECS", bad));
        assert!(
            matches!(
                load(&refused).map(|_| ()),
                Err(Error::InvalidConfig {
                    key: "TESSARIDB_S3_UPLOAD_MAX_AGE_SECS",
                    ..
                })
            ),
            "{bad}"
        );
    }
}

#[test]
fn usage_is_measured_every_minute_unless_configured_and_never_every_zero_seconds() {
    let config = load(&CREDENTIALS).expect("loads");
    assert_eq!(config.usage_interval_secs, 60);
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_USAGE_INTERVAL_SECS", "600"));
    assert_eq!(load(&vars).expect("loads").usage_interval_secs, 600);
    for bad in ["0", "-1", "hourly"] {
        let mut refused = CREDENTIALS.to_vec();
        refused.push(("TESSARIDB_S3_USAGE_INTERVAL_SECS", bad));
        assert!(
            matches!(
                load(&refused).map(|_| ()),
                Err(Error::InvalidConfig {
                    key: "TESSARIDB_S3_USAGE_INTERVAL_SECS",
                    ..
                })
            ),
            "{bad}"
        );
    }
}

#[test]
fn a_metadata_statement_waits_thirty_seconds_unless_configured_and_never_zero() {
    let waits =
        |vars: &[(&str, &str)]| load(vars).map(|config| config.meta.statement_timeout.as_secs());
    assert_eq!(waits(&CREDENTIALS).expect("loads"), 30);
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_META_STATEMENT_TIMEOUT_SECS", "5"));
    assert_eq!(waits(&vars).expect("loads"), 5);
    for bad in ["0", "-1", "soon"] {
        let mut refused = CREDENTIALS.to_vec();
        refused.push(("TESSARIDB_S3_META_STATEMENT_TIMEOUT_SECS", bad));
        assert!(
            matches!(
                waits(&refused),
                Err(Error::InvalidConfig {
                    key: "TESSARIDB_S3_META_STATEMENT_TIMEOUT_SECS",
                    ..
                })
            ),
            "{bad}"
        );
    }
}

pub(crate) fn with(vars: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut all = CREDENTIALS.to_vec();
    for (key, value) in vars {
        all.retain(|(k, _)| k != key);
        all.push((key, value));
    }
    all
}

#[test]
fn the_metadata_store_is_one_or_more_nodes() {
    let one = load(&CREDENTIALS).expect("loads");
    assert_eq!(one.meta.addresses, ["127.0.0.1:9080"]);
    assert_eq!(
        one.meta.replication,
        Replication::None,
        "one node keeps one copy unless told otherwise"
    );
    let three = load(&with(&[
        ("TESSARIDB_S3_META_ADDRESS", " a:1, b:2 ,c:3"),
        ("TESSARIDB_S3_META_REPLICATION", "3"),
    ]))
    .expect("loads");
    assert_eq!(three.meta.addresses, ["a:1", "b:2", "c:3"]);
    assert_eq!(three.meta.replication.clause(), "REPLICATION FACTOR 3");
    assert_eq!(
        load(&with(&[("TESSARIDB_S3_META_ADDRESS", " , ")])).map(|_| ()),
        Err(Error::MissingConfig {
            key: "TESSARIDB_S3_META_ADDRESS"
        })
    );
}

#[test]
fn several_metadata_nodes_need_their_replication_said() {
    let unsaid = load(&with(&[("TESSARIDB_S3_META_ADDRESS", "a:1,b:2")])).map(|_| ());
    assert!(
        matches!(
            unsaid,
            Err(Error::InvalidConfig {
                key: "TESSARIDB_S3_META_REPLICATION",
                ..
            })
        ),
        "{unsaid:?}"
    );
    let declined = load(&with(&[
        ("TESSARIDB_S3_META_ADDRESS", "a:1,b:2"),
        ("TESSARIDB_S3_META_REPLICATION", "none"),
    ]))
    .expect("loads");
    assert_eq!(declined.meta.replication.clause(), "REPLICATION NONE");
    for bad in ["0", "two", "-3", "256", "nothing"] {
        let refused = load(&with(&[("TESSARIDB_S3_META_REPLICATION", bad)])).map(|_| ());
        assert!(
            matches!(
                refused,
                Err(Error::InvalidConfig {
                    key: "TESSARIDB_S3_META_REPLICATION",
                    ..
                })
            ),
            "{bad}: {refused:?}"
        );
    }
}

#[test]
fn two_image_workers_run_unless_configured_and_zero_renders_nothing() {
    assert_eq!(load(&CREDENTIALS).expect("loads").media_workers, 2);
    for (given, workers) in [("0", 0), ("8", 8)] {
        let mut vars = CREDENTIALS.to_vec();
        vars.push(("TESSARIDB_S3_MEDIA_WORKERS", given));
        assert_eq!(load(&vars).expect("loads").media_workers, workers);
    }
    for bad in ["-1", "many", "65"] {
        let mut refused = CREDENTIALS.to_vec();
        refused.push(("TESSARIDB_S3_MEDIA_WORKERS", bad));
        assert!(
            matches!(
                load(&refused).map(|_| ()),
                Err(Error::InvalidConfig {
                    key: "TESSARIDB_S3_MEDIA_WORKERS",
                    ..
                })
            ),
            "{bad}"
        );
    }
}

#[test]
fn video_renditions_need_ffmpeg_named_and_a_blank_name_is_unset() {
    assert_eq!(load(&CREDENTIALS).expect("loads").ffmpeg, None);
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_FFMPEG", " /usr/bin/ffmpeg "));
    assert_eq!(
        load(&vars).expect("loads").ffmpeg,
        Some(std::path::PathBuf::from("/usr/bin/ffmpeg"))
    );
    let mut blank = CREDENTIALS.to_vec();
    blank.push(("TESSARIDB_S3_FFMPEG", "  "));
    assert_eq!(load(&blank).expect("loads").ffmpeg, None);
}

#[test]
fn a_region_that_cannot_name_its_database_is_refused() {
    for region in ["EU", "eu_west", "eu-", &format!("eu-{}", "a".repeat(60))] {
        let mut vars = CREDENTIALS.to_vec();
        vars.push(("TESSARIDB_S3_REGION", region));
        let refusal = load(&vars).map(|_| ()).expect_err(region);
        assert!(
            matches!(
                refusal,
                Error::InvalidConfig {
                    key: "TESSARIDB_S3_REGION",
                    ..
                }
            ),
            "{region}: {refusal:?}"
        );
    }
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_REGION", "eu-west-1"));
    assert_eq!(load(&vars).expect("loads").region, "eu-west-1");
}

#[test]
fn the_advertised_endpoint_is_an_http_url_naming_a_host() {
    let with = |value: &str| {
        let mut vars = CREDENTIALS.to_vec();
        vars.push(("TESSARIDB_S3_ADVERTISE", value));
        load(&vars)
    };
    assert_eq!(load(&CREDENTIALS).expect("loads").advertise, None);
    for (given, kept) in [
        ("https://s3.eu.example.test", "https://s3.eu.example.test"),
        ("http://127.0.0.1:9100/", "http://127.0.0.1:9100"),
    ] {
        assert_eq!(with(given).expect("loads").advertise.as_deref(), Some(kept));
    }
    for refused in [
        "s3.example.test",
        "ftp://host",
        "https://",
        "https://host/path",
    ] {
        assert!(
            matches!(
                with(refused),
                Err(Error::InvalidConfig {
                    key: "TESSARIDB_S3_ADVERTISE",
                    ..
                })
            ),
            "{refused}"
        );
    }
}

#[test]
fn the_s3_listener_speaks_tls_only_with_both_files_and_says_which_one_is_missing() {
    assert!(load(&CREDENTIALS).expect("loads").tls.is_none());
    let dir = std::env::temp_dir().join(format!("s3-tls-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
    std::fs::write(&cert, b"certificate bytes").expect("cert");
    std::fs::write(&key, b"key bytes").expect("key");
    let (cert, key) = (cert.to_str().expect("utf-8"), key.to_str().expect("utf-8"));
    let with = |extra: &[(&'static str, &str)]| {
        let mut vars: Vec<(&str, &str)> = CREDENTIALS.to_vec();
        vars.extend_from_slice(extra);
        load(&vars)
    };
    let both = with(&[
        ("TESSARIDB_S3_TLS_CERT", cert),
        ("TESSARIDB_S3_TLS_KEY", key),
    ])
    .expect("loads");
    assert_eq!(
        both.tls.expect("tls").certificate_pem,
        b"certificate bytes".to_vec()
    );
    assert!(matches!(
        with(&[("TESSARIDB_S3_TLS_CERT", cert)]).map(|_| ()),
        Err(Error::MissingConfig {
            key: "TESSARIDB_S3_TLS_KEY"
        })
    ));
    assert!(matches!(
        with(&[("TESSARIDB_S3_TLS_KEY", key)]).map(|_| ()),
        Err(Error::MissingConfig {
            key: "TESSARIDB_S3_TLS_CERT"
        })
    ));
    assert!(matches!(
        with(&[
            ("TESSARIDB_S3_TLS_CERT", "/nonexistent/cert.pem"),
            ("TESSARIDB_S3_TLS_KEY", key)
        ])
        .map(|_| ()),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_TLS_CERT",
            reason: "not a readable file"
        })
    ));
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn the_metrics_listener_is_off_unless_an_address_is_given() {
    assert_eq!(load(&CREDENTIALS).expect("loads").metrics_listen, None);
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_METRICS_LISTEN", "127.0.0.1:9102"));
    assert_eq!(
        load(&vars)
            .expect("loads")
            .metrics_listen
            .map(|addr| addr.to_string()),
        Some("127.0.0.1:9102".to_owned())
    );
    vars.pop();
    vars.push(("TESSARIDB_S3_METRICS_LISTEN", "everywhere"));
    let refusal = load(&vars).map(|_| ()).expect_err("refused");
    assert!(
        matches!(
            refusal,
            Error::InvalidConfig {
                key: "TESSARIDB_S3_METRICS_LISTEN",
                ..
            }
        ),
        "{refusal:?}"
    );
}
