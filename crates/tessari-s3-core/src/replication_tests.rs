use super::{Configuration, ReplicationError, Rule, destination_bucket, matching, validate};

fn rule(id: &str, priority: u32, prefix: &str, destination: &str) -> Rule {
    Rule {
        id: id.to_owned(),
        enabled: true,
        priority,
        prefix: prefix.to_owned(),
        destination: destination.to_owned(),
        delete_markers: false,
    }
}

fn configuration(rules: Vec<Rule>) -> Configuration {
    Configuration {
        role: "arn:aws:iam::000000000000:role/replication".to_owned(),
        rules,
    }
}

#[test]
fn a_destination_is_named_by_its_bucket_arn_and_nothing_else() {
    assert_eq!(destination_bucket("arn:aws:s3:::backup"), Some("backup"));
    assert_eq!(destination_bucket("backup"), None);
    assert_eq!(destination_bucket("arn:aws:s3:::"), None);
    assert_eq!(destination_bucket("arn:aws:s3:::a/b"), None);
    assert_eq!(destination_bucket("arn:aws:s3:eu-west-1::backup"), None);
}

#[test]
fn a_configuration_of_distinct_rules_to_other_buckets_is_accepted() {
    let rules = vec![rule("a", 1, "logs/", "backup"), rule("b", 2, "", "archive")];
    assert_eq!(validate(&configuration(rules), "source"), Ok(()));
}

#[test]
fn each_refusal_names_what_s3_refuses() {
    let cases = [
        (configuration(Vec::new()), ReplicationError::NoRules),
        (
            Configuration {
                role: String::new(),
                ..configuration(vec![rule("a", 1, "", "backup")])
            },
            ReplicationError::NoRole,
        ),
        (
            configuration(vec![rule("a", 1, "", "backup"), rule("a", 2, "", "backup")]),
            ReplicationError::DuplicateId,
        ),
        (
            configuration(vec![
                rule("a", 1, "", "backup"),
                rule("b", 1, "x", "backup"),
            ]),
            ReplicationError::DuplicatePriority,
        ),
        (
            configuration(vec![rule("a", 1, "", "source")]),
            ReplicationError::SameBucket,
        ),
        (
            configuration(vec![rule(&"i".repeat(256), 1, "", "backup")]),
            ReplicationError::IdTooLong,
        ),
    ];
    for (config, refused) in cases {
        assert_eq!(validate(&config, "source"), Err(refused));
    }
    let many = (0..1001)
        .map(|n| rule(&format!("r{n}"), n, "", "backup"))
        .collect();
    assert_eq!(
        validate(&configuration(many), "source"),
        Err(ReplicationError::TooManyRules)
    );
}

#[test]
fn a_key_is_sent_by_the_highest_priority_enabled_rule_whose_prefix_it_has() {
    let mut off = rule("off", 9, "logs/", "never");
    off.enabled = false;
    let rules = [
        rule("all", 1, "", "everything"),
        rule("logs", 5, "logs/", "logs-copy"),
        off,
    ];
    assert_eq!(
        matching(&rules, "logs/today").map(|r| r.id.as_str()),
        Some("logs")
    );
    assert_eq!(
        matching(&rules, "photos/cat").map(|r| r.id.as_str()),
        Some("all")
    );
    assert_eq!(
        matching(&rules[1..], "photos/cat").map(|r| r.id.as_str()),
        None
    );
}
