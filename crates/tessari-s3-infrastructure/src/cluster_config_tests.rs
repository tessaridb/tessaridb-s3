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

#[test]
fn healing_runs_every_minute_unless_told_otherwise() {
    let cluster = |vars: &[(&'static str, &'static str)]| {
        load(vars).map(|config| config.cluster.expect("clustered").heal_interval_secs)
    };
    assert_eq!(cluster(&with(&CLUSTER)), Ok(60));
    let mut set = with(&CLUSTER);
    set.push(("TESSARIDB_S3_HEAL_INTERVAL_SECS", "15"));
    assert_eq!(cluster(&set), Ok(15));
    let mut zero = with(&CLUSTER);
    zero.push(("TESSARIDB_S3_HEAL_INTERVAL_SECS", "0"));
    assert!(matches!(
        cluster(&zero),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_HEAL_INTERVAL_SECS",
            ..
        })
    ));
}

#[test]
fn internal_tls_is_three_files_read_together_or_none() {
    let dir = std::env::temp_dir().join(format!("tessari-s3-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = |name: &str, body: &str| {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("written");
        path.to_string_lossy().into_owned()
    };
    let (cert, key, ca) = (
        file("cert.pem", "CERT"),
        file("key.pem", "PRIVATE-KEY-BYTES"),
        file("ca.pem", "AUTHORITY"),
    );
    let tls = [
        ("TESSARIDB_S3_INTERNAL_TLS_CERT", cert.as_str()),
        ("TESSARIDB_S3_INTERNAL_TLS_KEY", key.as_str()),
        ("TESSARIDB_S3_INTERNAL_TLS_CA", ca.as_str()),
    ];
    let cluster = |extra: &[(&str, &str)]| {
        let mut vars: Vec<(&str, &str)> = with(&CLUSTER);
        vars.extend_from_slice(extra);
        load(&vars).map(|config| config.cluster.expect("clustered"))
    };
    assert!(
        cluster(&[]).expect("loads").tls.is_none(),
        "plain unless asked"
    );
    let read = cluster(&tls).expect("loads").tls.expect("tls");
    assert_eq!(read.certificate_pem, b"CERT");
    assert_eq!(read.authority_pem, b"AUTHORITY");
    assert_eq!(read.key_pem(), b"PRIVATE-KEY-BYTES");
    assert!(
        !format!("{read:?}").contains("PRIVATE-KEY"),
        "the key is never printed"
    );
    for (index, (missing, _)) in tls.iter().enumerate() {
        let partial: Vec<(&'static str, &str)> = tls
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != index)
            .map(|(_, pair)| *pair)
            .collect();
        assert!(
            matches!(cluster(&partial), Err(Error::MissingConfig { key }) if key == *missing),
            "{missing} missing"
        );
    }
    let unreadable = [
        tls[0],
        tls[1],
        ("TESSARIDB_S3_INTERNAL_TLS_CA", "/nonexistent/ca.pem"),
    ];
    assert!(matches!(
        cluster(&unreadable),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_INTERNAL_TLS_CA",
            ..
        })
    ));
    let mut stray = with(&CREDENTIALS);
    stray.push(tls[0]);
    assert!(
        matches!(
            load(&stray),
            Err(Error::InvalidConfig {
                key: "TESSARIDB_S3_ERASURE",
                ..
            })
        ),
        "TLS for a node that is not clustered is refused, not ignored"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}
