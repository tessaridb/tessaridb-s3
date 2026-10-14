use crate::Error;
use crate::config::tests::{CREDENTIALS, load};

fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut vars = CREDENTIALS.to_vec();
    vars.extend_from_slice(extra);
    vars
}

#[test]
fn no_listen_address_means_no_console() {
    assert!(load(&CREDENTIALS).expect("loads").console.is_none());
}

#[test]
fn a_listen_address_serves_the_console_in_the_clear_with_default_limits() {
    let config = load(&with(&[("TESSARIDB_S3_CONSOLE_LISTEN", "127.0.0.1:9101")])).expect("loads");
    let console = config.console.expect("a console");
    assert_eq!(console.listen.to_string(), "127.0.0.1:9101");
    assert!(console.tls.is_none());
    assert_eq!(
        (console.sign_ins_per_minute, console.requests_per_minute),
        (10, 600)
    );
}

#[test]
fn limits_are_configurable_and_never_zero() {
    let config = load(&with(&[
        ("TESSARIDB_S3_CONSOLE_LISTEN", "127.0.0.1:9101"),
        ("TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE", "3"),
        ("TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE", "50"),
    ]))
    .expect("loads");
    let console = config.console.expect("a console");
    assert_eq!(
        (console.sign_ins_per_minute, console.requests_per_minute),
        (3, 50)
    );
    let zero = load(&with(&[
        ("TESSARIDB_S3_CONSOLE_LISTEN", "127.0.0.1:9101"),
        ("TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE", "0"),
    ]));
    assert!(matches!(
        zero.map(|_| ()),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE",
            ..
        })
    ));
}

#[test]
fn a_bad_address_or_half_a_tls_pair_is_refused() {
    let bad = load(&with(&[("TESSARIDB_S3_CONSOLE_LISTEN", "everywhere")]));
    assert!(matches!(
        bad.map(|_| ()),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_CONSOLE_LISTEN",
            ..
        })
    ));
    let half = load(&with(&[
        ("TESSARIDB_S3_CONSOLE_LISTEN", "127.0.0.1:9101"),
        ("TESSARIDB_S3_CONSOLE_TLS_CERT", "/nonexistent/cert.pem"),
    ]));
    assert!(matches!(
        half.map(|_| ()),
        Err(Error::MissingConfig {
            key: "TESSARIDB_S3_CONSOLE_TLS_KEY"
        })
    ));
}

#[test]
fn console_settings_without_a_listen_address_are_refused_rather_than_ignored() {
    for key in [
        "TESSARIDB_S3_CONSOLE_TLS_CERT",
        "TESSARIDB_S3_CONSOLE_TLS_KEY",
        "TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE",
        "TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE",
    ] {
        let stray = load(&with(&[(key, "1")]));
        assert!(
            matches!(stray.map(|_| ()), Err(Error::InvalidConfig { key: found, .. }) if found == key),
            "{key}"
        );
    }
}

const OIDC: [(&str, &str); 5] = [
    ("TESSARIDB_S3_CONSOLE_LISTEN", "127.0.0.1:9101"),
    (
        "TESSARIDB_S3_CONSOLE_OIDC_ISSUER",
        "https://id.example.test/realms/ops",
    ),
    (
        "TESSARIDB_S3_CONSOLE_OIDC_CLIENT_ID",
        "tessaridb-s3-console",
    ),
    (
        "TESSARIDB_S3_CONSOLE_OIDC_CLIENT_SECRET",
        "client-secret-fixture-0123456789",
    ),
    (
        "TESSARIDB_S3_CONSOLE_OIDC_REDIRECT_URI",
        "https://console.example.test/api/v1/auth/oidc/callback",
    ),
];

#[test]
fn an_identity_provider_is_read_whole_and_requires_mfa_by_default() {
    let config = load(&with(&OIDC)).expect("loads");
    let oidc = config
        .console
        .expect("a console")
        .oidc
        .expect("an identity provider");
    assert_eq!(oidc.issuer, "https://id.example.test/realms/ops");
    assert_eq!(oidc.client_id, "tessaridb-s3-console");
    assert_eq!(
        oidc.client_secret.expose(),
        "client-secret-fixture-0123456789"
    );
    assert_eq!(
        oidc.redirect_uri,
        "https://console.example.test/api/v1/auth/oidc/callback"
    );
    assert_eq!(oidc.mfa_amr, vec!["mfa".to_owned()]);
    let mut chosen = with(&OIDC);
    chosen.push(("TESSARIDB_S3_CONSOLE_OIDC_MFA_AMR", "hwk, otp"));
    let oidc = load(&chosen)
        .expect("loads")
        .console
        .expect("console")
        .oidc
        .expect("provider");
    assert_eq!(oidc.mfa_amr, vec!["hwk".to_owned(), "otp".to_owned()]);
}

#[test]
fn each_identity_provider_variable_is_named_when_missing() {
    for (missing, _) in OIDC.iter().skip(2) {
        let vars: Vec<_> = with(&OIDC)
            .into_iter()
            .filter(|(k, _)| k != missing)
            .collect();
        match load(&vars).map(|config| config.console.map(|console| console.oidc.is_some())) {
            Err(Error::MissingConfig { key }) => assert_eq!(key, *missing),
            other => panic!("{missing}: {other:?}"),
        }
    }
    // A companion without the issuer is refused rather than ignored.
    let vars: Vec<_> = with(&OIDC)
        .into_iter()
        .filter(|(k, _)| *k != "TESSARIDB_S3_CONSOLE_OIDC_ISSUER")
        .collect();
    assert!(matches!(
        load(&vars).map(|_| ()),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_CONSOLE_OIDC_CLIENT_ID",
            ..
        })
    ));
}

#[test]
fn an_identity_provider_is_reached_over_https_except_on_this_machine() {
    for (key, value) in [
        (
            "TESSARIDB_S3_CONSOLE_OIDC_ISSUER",
            "http://id.example.test/realms/ops",
        ),
        (
            "TESSARIDB_S3_CONSOLE_OIDC_REDIRECT_URI",
            "http://console.example.test/callback",
        ),
        ("TESSARIDB_S3_CONSOLE_OIDC_ISSUER", "not a url"),
    ] {
        let mut vars: Vec<_> = with(&OIDC).into_iter().filter(|(k, _)| *k != key).collect();
        vars.push((key, value));
        let error = load(&vars).map(|_| ()).expect_err(value);
        assert!(
            matches!(error, Error::InvalidConfig { key: k, .. } if k == key),
            "{error:?}"
        );
        assert!(
            !error.to_string().contains(value),
            "the value is not echoed: {error}"
        );
    }
    let mut loopback: Vec<_> = with(&OIDC)
        .into_iter()
        .filter(|(k, _)| !k.ends_with("_ISSUER") && !k.ends_with("_REDIRECT_URI"))
        .collect();
    loopback.push(("TESSARIDB_S3_CONSOLE_OIDC_ISSUER", "http://127.0.0.1:47950"));
    loopback.push((
        "TESSARIDB_S3_CONSOLE_OIDC_REDIRECT_URI",
        "http://localhost:47951/api/v1/auth/oidc/callback",
    ));
    assert!(
        load(&loopback)
            .expect("loopback over http")
            .console
            .expect("console")
            .oidc
            .is_some()
    );
}

#[test]
fn an_identity_provider_without_a_console_is_refused() {
    let vars: Vec<_> = with(&OIDC)
        .into_iter()
        .filter(|(k, _)| *k != "TESSARIDB_S3_CONSOLE_LISTEN")
        .collect();
    assert!(matches!(
        load(&vars).map(|_| ()),
        Err(Error::InvalidConfig {
            reason: "set without TESSARIDB_S3_CONSOLE_LISTEN",
            ..
        })
    ));
}
