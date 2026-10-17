use crate::Error;
use crate::config::tests::{CREDENTIALS, load};

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

#[test]
fn the_access_log_is_on_unless_switched_off() {
    assert!(load(&CREDENTIALS).expect("loads").audit);
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_AUDIT", "off"));
    assert!(!load(&vars).expect("loads").audit);
    vars.pop();
    vars.push(("TESSARIDB_S3_AUDIT", "no"));
    let refusal = load(&vars).map(|_| ()).expect_err("refused");
    assert!(
        matches!(
            refusal,
            Error::InvalidConfig {
                key: "TESSARIDB_S3_AUDIT",
                ..
            }
        ),
        "{refusal:?}"
    );
}
