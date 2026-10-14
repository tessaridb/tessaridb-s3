use super::{NotificationError, Rule, arn, target_of, validate};

fn rule(id: &str, events: &[&str], prefix: &str, suffix: &str) -> Rule {
    Rule {
        id: id.to_owned(),
        target: "hooks".to_owned(),
        events: events.iter().map(|event| (*event).to_owned()).collect(),
        prefix: prefix.to_owned(),
        suffix: suffix.to_owned(),
    }
}

fn declared(target: &str) -> bool {
    target == "hooks"
}

#[test]
fn an_arn_names_a_declared_target_and_reads_back() {
    assert_eq!(arn("hooks"), "arn:tessaridb:sqs:::hooks");
    assert_eq!(target_of("arn:tessaridb:sqs:::hooks"), Some("hooks"));
    assert_eq!(target_of("arn:aws:sqs:us-east-1:123456789012:hooks"), None);
    assert_eq!(target_of("arn:tessaridb:sqs:::"), None);
}

#[test]
fn rules_on_disjoint_events_or_keys_are_accepted() {
    let rules = [
        rule("created", &["s3:ObjectCreated:*"], "", ""),
        rule("removed", &["s3:ObjectRemoved:Delete"], "logs/", ""),
        rule(
            "removed-images",
            &["s3:ObjectRemoved:Delete"],
            "images/",
            ".jpg",
        ),
        rule("expired", &["s3:LifecycleExpiration:*"], "", ""),
    ];
    assert_eq!(validate(&rules, declared), Ok(()));
    assert_eq!(
        validate(&[], declared),
        Ok(()),
        "an empty configuration turns notifications off"
    );
}

#[test]
fn what_a_configuration_may_not_say_is_refused_by_name() {
    assert_eq!(
        validate(&[rule("a", &["s3:ObjectRestore:Post"], "", "")], declared),
        Err(NotificationError::UnsupportedEvent(
            "s3:ObjectRestore:Post".to_owned()
        ))
    );
    assert_eq!(
        validate(&[rule("a", &[], "", "")], declared),
        Err(NotificationError::NoEvent)
    );
    let mut elsewhere = rule("a", &["s3:ObjectCreated:Put"], "", "");
    elsewhere.target = "nowhere".to_owned();
    assert_eq!(
        validate(&[elsewhere], declared),
        Err(NotificationError::UndeclaredTarget(
            "arn:tessaridb:sqs:::nowhere".to_owned()
        ))
    );
    assert_eq!(
        validate(
            &[
                rule("a", &["s3:ObjectCreated:Put"], "", ""),
                rule("a", &["s3:ObjectRemoved:Delete"], "", ""),
            ],
            declared
        ),
        Err(NotificationError::DuplicateId)
    );
    assert_eq!(
        validate(
            &[rule(&"x".repeat(256), &["s3:ObjectCreated:Put"], "", "")],
            declared
        ),
        Err(NotificationError::IdTooLong)
    );
}

#[test]
fn rules_sharing_an_event_on_overlapping_keys_are_refused() {
    // A wildcard covers its family; a prefix overlaps any prefix it starts or ends; a suffix likewise.
    let overlaps = [
        (
            ["s3:ObjectCreated:*", "s3:ObjectCreated:Put"],
            ["", ""],
            ["", ""],
        ),
        (
            ["s3:ObjectCreated:Put", "s3:ObjectCreated:Put"],
            ["logs/", "logs/2026/"],
            ["", ""],
        ),
        (
            ["s3:ObjectRemoved:Delete", "s3:ObjectRemoved:Delete"],
            ["", ""],
            [".jpg", "x.jpg"],
        ),
    ];
    for (events, prefixes, suffixes) in overlaps {
        let rules = [
            rule("one", &[events[0]], prefixes[0], suffixes[0]),
            rule("two", &[events[1]], prefixes[1], suffixes[1]),
        ];
        assert_eq!(
            validate(&rules, declared),
            Err(NotificationError::Overlap),
            "{events:?} {prefixes:?} {suffixes:?}"
        );
    }
    // Control: the same events on keys that cannot both match are accepted.
    let apart = [
        rule("one", &["s3:ObjectCreated:*"], "logs/", ""),
        rule("two", &["s3:ObjectCreated:Put"], "images/", ""),
    ];
    assert_eq!(validate(&apart, declared), Ok(()));
}
