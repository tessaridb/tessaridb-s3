//! A bucket's CORS rules: what a configuration may say, and which rule answers a preflight or an actual request.

use super::cors::{CorsError, Method, Rule, actual, allowed_origin, preflight, validate};

fn rule(origins: &[&str], methods: &[Method], headers: &[&str]) -> Rule {
    Rule {
        id: None,
        origins: origins.iter().map(|o| (*o).to_owned()).collect(),
        methods: methods.to_vec(),
        headers: headers.iter().map(|h| (*h).to_owned()).collect(),
        expose: Vec::new(),
        max_age: None,
    }
}

#[test]
fn a_configuration_is_refused_for_each_shape_s3_refuses() {
    let good = rule(&["https://*.example.com"], &[Method::Get], &["x-amz-*"]);
    assert_eq!(validate(std::slice::from_ref(&good)), Ok(()));
    assert_eq!(validate(&[]), Err(CorsError::NoRules));
    assert_eq!(
        validate(&vec![good.clone(); 101]),
        Err(CorsError::TooManyRules)
    );
    let cases = [
        (rule(&[], &[Method::Get], &[]), CorsError::NoOrigin),
        (rule(&["*"], &[], &[]), CorsError::NoMethod),
        (
            rule(&["https://*.*.example.com"], &[Method::Get], &[]),
            CorsError::TwoWildcards,
        ),
        (
            rule(&["*"], &[Method::Get], &["x-*-*"]),
            CorsError::TwoWildcards,
        ),
        (
            Rule {
                expose: vec!["x-amz-*".to_owned()],
                ..good.clone()
            },
            CorsError::WildcardExposed,
        ),
        (
            Rule {
                id: Some("x".repeat(256)),
                ..good.clone()
            },
            CorsError::LongId,
        ),
    ];
    for (bad, error) in cases {
        assert_eq!(validate(&[bad]), Err(error));
    }
    assert_eq!(Method::parse("PATCH"), None);
    assert_eq!(Method::parse("HEAD"), Some(Method::Head));
}

#[test]
fn a_preflight_is_answered_by_the_first_rule_matching_origin_method_and_every_header() {
    let rules = [
        rule(
            &["https://app.example.com"],
            &[Method::Put],
            &["content-type"],
        ),
        rule(
            &["https://*.example.com"],
            &[Method::Put, Method::Get],
            &["*"],
        ),
        rule(&["*"], &[Method::Get], &[]),
    ];
    let first = preflight(&rules, "https://app.example.com", "PUT", &["Content-Type"]);
    assert_eq!(first, Some(&rules[0]), "header names compare without case");
    // The first rule lacks a requested header, so the second answers.
    let second = preflight(
        &rules,
        "https://app.example.com",
        "PUT",
        &["content-type", "x-amz-meta-a"],
    );
    assert_eq!(second, Some(&rules[1]));
    assert_eq!(
        preflight(&rules, "https://other.org", "GET", &[]),
        Some(&rules[2])
    );
    assert_eq!(preflight(&rules, "https://other.org", "PUT", &[]), None);
    assert_eq!(
        preflight(&rules, "https://other.org", "GET", &["x-amz-date"]),
        None,
        "a rule naming no header allows none"
    );
    // Origins compare with their case; an unknown method matches nothing.
    assert_eq!(
        preflight(&rules[..1], "https://APP.example.com", "PUT", &[]),
        None
    );
    assert_eq!(preflight(&rules, "https://other.org", "PATCH", &[]), None);
}

#[test]
fn an_allowed_origin_is_the_star_only_when_the_matching_pattern_is() {
    let open = rule(&["*"], &[Method::Get], &[]);
    let named = rule(&["https://*.example.com"], &[Method::Get], &[]);
    assert_eq!(allowed_origin(&open, "https://a.org"), "*");
    assert_eq!(
        allowed_origin(&named, "https://a.example.com"),
        "https://a.example.com"
    );
}

#[test]
fn an_actual_request_matches_origin_and_method_only() {
    let rules = [rule(
        &["https://app.example.com"],
        &[Method::Get],
        &["content-type"],
    )];
    assert_eq!(
        actual(&rules, "https://app.example.com", "GET"),
        Some(&rules[0])
    );
    assert_eq!(actual(&rules, "https://app.example.com", "DELETE"), None);
    assert_eq!(actual(&rules, "https://evil.example.com", "GET"), None);
}
