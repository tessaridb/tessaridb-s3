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
