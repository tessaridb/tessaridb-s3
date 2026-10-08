use crate::Error;
use crate::config::tests::{CREDENTIALS, load, with};

const CLUSTER: [(&str, &str); 6] = [
    ("TESSARIDB_S3_ERASURE", "4+2"),
    ("TESSARIDB_S3_NODE_ID", "n1"),
    ("TESSARIDB_S3_INTERNAL_LISTEN", "127.0.0.1:47970"),
    ("TESSARIDB_S3_INTERNAL_ADVERTISE", "n1.internal:47970"),
    (
        "TESSARIDB_S3_CLUSTER_SECRET",
        "cluster-secret-0123456789abcdef0123",
    ),
    ("TESSARIDB_S3_DATA_DIR", "/srv/s3"),
];

#[test]
fn a_node_without_an_erasure_code_is_not_clustered() {
    assert!(load(&CREDENTIALS).expect("loads").cluster.is_none());
}

#[test]
fn the_cluster_settings_are_read_together() {
    let cluster = load(&with(&CLUSTER))
        .expect("loads")
        .cluster
        .expect("clustered");
    assert_eq!((cluster.code.data(), cluster.code.parity()), (4, 2));
    assert_eq!(cluster.node.as_str(), "n1");
    assert_eq!(cluster.internal_listen.to_string(), "127.0.0.1:47970");
    assert_eq!(cluster.internal_advertise, "n1.internal:47970");
    let printed = format!("{cluster:?}");
    assert!(!printed.contains("cluster-secret"), "{printed}");
}

#[test]
fn a_partial_or_invalid_cluster_setting_is_refused_by_name() {
    for missing in [
        "TESSARIDB_S3_NODE_ID",
        "TESSARIDB_S3_INTERNAL_LISTEN",
        "TESSARIDB_S3_INTERNAL_ADVERTISE",
        "TESSARIDB_S3_CLUSTER_SECRET",
        "TESSARIDB_S3_DATA_DIR",
    ] {
        let mut vars = with(&CLUSTER);
        vars.retain(|(k, _)| *k != missing);
        assert_eq!(
            load(&vars).map(|_| ()),
            Err(Error::MissingConfig { key: missing }),
            "{missing}"
        );
    }
    let mut no_code = with(&CLUSTER);
    no_code.retain(|(k, _)| *k != "TESSARIDB_S3_ERASURE");
    assert!(matches!(
        load(&no_code).map(|_| ()),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_ERASURE",
            ..
        })
    ));
    for (key, bad) in [
        ("TESSARIDB_S3_ERASURE", "4+0"),
        ("TESSARIDB_S3_ERASURE", "four"),
        ("TESSARIDB_S3_NODE_ID", "N1"),
        ("TESSARIDB_S3_INTERNAL_LISTEN", "nowhere"),
        ("TESSARIDB_S3_CLUSTER_SECRET", "short-secret"),
    ] {
        let mut vars = with(&CLUSTER);
        vars.retain(|(k, _)| *k != key);
        vars.push((key, bad));
        assert!(
            matches!(load(&vars).map(|_| ()), Err(Error::InvalidConfig { key: k, .. }) if k == key),
            "{key}={bad}"
        );
    }
}
