use super::{ReadConditions, ReadVerdict, etag_list_matches, evaluate};
use tessari_s3_types::Timestamp;

const ETAG: &str = "\"abc\"";
/// Mon, 12 Oct 2009 17:50:30 GMT.
const MODIFIED: Timestamp = Timestamp {
    seconds: 1_255_369_830,
    nanos: 500_000_000,
};
const BEFORE: &str = "Mon, 12 Oct 2009 17:50:29 GMT";
const AT: &str = "Mon, 12 Oct 2009 17:50:30 GMT";
const AFTER: &str = "Mon, 12 Oct 2009 17:50:31 GMT";

fn verdict(conditions: ReadConditions<'_>) -> ReadVerdict {
    evaluate(&conditions, ETAG, MODIFIED)
}

#[test]
fn each_condition_alone() {
    assert_eq!(verdict(ReadConditions::default()), ReadVerdict::Proceed);
    assert_eq!(
        verdict(ReadConditions {
            if_match: Some("\"abc\""),
            ..Default::default()
        }),
        ReadVerdict::Proceed
    );
    assert_eq!(
        verdict(ReadConditions {
            if_match: Some("\"xyz\""),
            ..Default::default()
        }),
        ReadVerdict::PreconditionFailed
    );
    assert_eq!(
        verdict(ReadConditions {
            if_none_match: Some("\"abc\""),
            ..Default::default()
        }),
        ReadVerdict::NotModified
    );
    assert_eq!(
        verdict(ReadConditions {
            if_none_match: Some("\"xyz\""),
            ..Default::default()
        }),
        ReadVerdict::Proceed
    );
    assert_eq!(
        verdict(ReadConditions {
            if_modified_since: Some(AT),
            ..Default::default()
        }),
        ReadVerdict::NotModified
    );
    assert_eq!(
        verdict(ReadConditions {
            if_modified_since: Some(BEFORE),
            ..Default::default()
        }),
        ReadVerdict::Proceed
    );
    assert_eq!(
        verdict(ReadConditions {
            if_unmodified_since: Some(AT),
            ..Default::default()
        }),
        ReadVerdict::Proceed
    );
    assert_eq!(
        verdict(ReadConditions {
            if_unmodified_since: Some(BEFORE),
            ..Default::default()
        }),
        ReadVerdict::PreconditionFailed
    );
    assert_eq!(
        verdict(ReadConditions {
            if_modified_since: Some(AFTER),
            ..Default::default()
        }),
        ReadVerdict::NotModified
    );
}

#[test]
fn s3_precedence_between_the_pairs() {
    // If-Match TRUE (the tag matches) and If-Unmodified-Since FALSE (modified after the date) → 200.
    let if_match_wins = ReadConditions {
        if_match: Some(ETAG),
        if_unmodified_since: Some(BEFORE),
        ..Default::default()
    };
    assert_eq!(verdict(if_match_wins), ReadVerdict::Proceed);
    // If-None-Match FALSE (the tag matches) and If-Modified-Since TRUE (modified after the date) → 304.
    let none_match_wins = ReadConditions {
        if_none_match: Some(ETAG),
        if_modified_since: Some(BEFORE),
        ..Default::default()
    };
    assert_eq!(verdict(none_match_wins), ReadVerdict::NotModified);
}

#[test]
fn an_unparseable_date_is_ignored() {
    let garbage = ReadConditions {
        if_unmodified_since: Some("yesterday"),
        ..Default::default()
    };
    assert_eq!(verdict(garbage), ReadVerdict::Proceed);
}

#[test]
fn entity_tag_lists() {
    assert!(etag_list_matches("*", ETAG));
    assert!(etag_list_matches("\"x\", \"abc\"", ETAG));
    assert!(
        etag_list_matches("W/\"abc\"", ETAG),
        "weak comparison for conditional reads"
    );
    assert!(
        etag_list_matches("abc", ETAG),
        "clients that drop the quotes still match"
    );
    assert!(!etag_list_matches("\"ab\"", ETAG));
}
