use super::{NotifyEndpoint, NotifyTarget};
use crate::Error;
use crate::config::tests::{CREDENTIALS, load};

fn with_targets(value: &str) -> Vec<(&str, &str)> {
    let mut vars = CREDENTIALS.to_vec();
    vars.push(("TESSARIDB_S3_NOTIFY_TARGETS", value));
    vars
}

fn refused(value: &str) -> &'static str {
    match load(&with_targets(value)).expect_err("refused") {
        Error::InvalidConfig { key, reason } => {
            assert_eq!(key, "TESSARIDB_S3_NOTIFY_TARGETS");
            reason
        }
        other => panic!("an invalid configuration, not {other:?}"),
    }
}

#[test]
fn without_the_variable_no_target_is_declared() {
    assert!(load(&CREDENTIALS).expect("loads").notify_targets.is_empty());
}

#[test]
fn webhook_and_kafka_targets_are_read_in_order() {
    let config = load(&with_targets(
        " hooks=webhook:https://events.example/s3 ; orders=kafka:k1:9092,k2:9092/s3-events ",
    ))
    .expect("loads");
    assert_eq!(
        config.notify_targets,
        vec![
            NotifyTarget {
                id: "hooks".to_owned(),
                endpoint: NotifyEndpoint::Webhook("https://events.example/s3".to_owned()),
            },
            NotifyTarget {
                id: "orders".to_owned(),
                endpoint: NotifyEndpoint::Kafka {
                    brokers: vec!["k1:9092".to_owned(), "k2:9092".to_owned()],
                    topic: "s3-events".to_owned(),
                },
            },
        ]
    );
}

#[test]
fn a_malformed_target_is_refused_by_what_is_wrong() {
    assert_eq!(
        refused("hooks"),
        "a target is <id>=<webhook|kafka>:<endpoint>"
    );
    assert_eq!(
        refused("bad id=webhook:https://e.example"),
        "a target id is 1 to 64 letters, digits, '-' or '_'"
    );
    assert_eq!(
        refused("a=webhook:https://e.example;a=webhook:https://f.example"),
        "two targets share an id"
    );
    assert_eq!(refused("a=queue:x"), "a target kind is webhook or kafka");
    assert_eq!(
        refused("a=webhook:ftp://e.example"),
        "a webhook is an http:// or https:// URL with a host"
    );
    assert_eq!(
        refused("a=webhook:https://"),
        "a webhook is an http:// or https:// URL with a host"
    );
    assert_eq!(
        refused("a=kafka:k1:9092"),
        "a kafka target is <host:port>[,<host:port>…]/<topic>"
    );
    assert_eq!(
        refused("a=kafka:k1/topic"),
        "a kafka target is <host:port>[,<host:port>…]/<topic>"
    );
}

#[test]
fn the_database_topic_s_id_is_not_an_operator_s_to_declare() {
    assert_eq!(
        refused("tessaridb=webhook:https://e.example"),
        "'tessaridb' names the database's own topic and cannot be declared"
    );
}
