use super::{Expiration, LifecycleError, Noncurrent, Rule, validate};

fn expiring(id: &str, days: u32) -> Rule {
    Rule {
        id: id.to_owned(),
        enabled: true,
        prefix: String::new(),
        expiration: Some(Expiration::Days(days)),
        expired_object_delete_marker: false,
        noncurrent: None,
        abort_after_days: None,
    }
}

#[test]
fn a_rule_with_one_positive_action_is_accepted() {
    assert_eq!(validate(&[expiring("a", 30)]), Ok(()));
    let rule = Rule {
        expiration: None,
        noncurrent: Some(Noncurrent {
            days: 1,
            keep: Some(3),
        }),
        ..expiring("b", 1)
    };
    assert_eq!(validate(&[rule]), Ok(()));
}

#[test]
fn zero_days_ids_too_long_or_repeated_and_actionless_rules_are_refused() {
    assert_eq!(
        validate(&[expiring("a", 0)]),
        Err(LifecycleError::NotPositive("Days"))
    );
    assert_eq!(
        validate(&[expiring(&"x".repeat(256), 1)]),
        Err(LifecycleError::IdTooLong)
    );
    assert_eq!(
        validate(&[expiring("same", 1), expiring("same", 2)]),
        Err(LifecycleError::DuplicateId)
    );
    let idle = Rule {
        expiration: None,
        ..expiring("idle", 1)
    };
    assert_eq!(validate(&[idle]), Err(LifecycleError::NoAction));
    assert_eq!(validate(&[]), Err(LifecycleError::NoRules));
}

#[test]
fn more_than_a_thousand_rules_and_an_out_of_range_keep_are_refused() {
    let many: Vec<Rule> = (0..1001).map(|n| expiring(&n.to_string(), 1)).collect();
    assert_eq!(validate(&many), Err(LifecycleError::TooManyRules));
    let keep = Rule {
        expiration: None,
        noncurrent: Some(Noncurrent {
            days: 1,
            keep: Some(101),
        }),
        ..expiring("k", 1)
    };
    assert_eq!(validate(&[keep]), Err(LifecycleError::KeepOutOfRange));
    let marker_with_days = Rule {
        expired_object_delete_marker: true,
        ..expiring("m", 1)
    };
    assert_eq!(
        validate(&[marker_with_days]),
        Err(LifecycleError::MarkerWithDaysOrDate)
    );
}
