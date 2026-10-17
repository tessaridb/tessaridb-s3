//! Whether a policy is public (S3 ref 04 M5): public until proven otherwise — every Allow must name its principals or
//! narrow them by a fixed source address or user name.

use super::super::model::SpaceName;
use super::{BucketPolicy, PublicAccessBlock};

fn policy(statements: &str) -> BucketPolicy {
    let text = format!(r#"{{"Version":"2012-10-17","Statement":[{statements}]}}"#);
    BucketPolicy::parse(&text, "photos", &SpaceName::new("acme").expect("space")).expect("parsed")
}

fn allow_anyone(condition: &str) -> String {
    format!(
        r#"{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"{condition}}}"#
    )
}

#[test]
fn an_allow_to_everyone_is_public_and_one_to_named_users_is_not() {
    assert!(policy(&allow_anyone("")).is_public());
    assert!(policy(r#"{"Effect":"Allow","Principal":{"AWS":"*"},"Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}"#).is_public());
    assert!(!policy(r#"{"Effect":"Allow","Principal":{"AWS":["arn:aws:iam::acme:user/ann","arn:aws:iam::acme:root"]},"Action":"s3:*","Resource":"arn:aws:s3:::photos/*"}"#).is_public());
    // A Deny to everyone opens nothing.
    assert!(!policy(r#"{"Effect":"Deny","Principal":"*","Action":"s3:*","Resource":"arn:aws:s3:::photos/*"}"#).is_public());
}

#[test]
fn one_public_statement_makes_the_whole_policy_public() {
    let named = r#"{"Effect":"Allow","Principal":{"AWS":"arn:aws:iam::acme:user/ann"},"Action":"s3:*","Resource":"arn:aws:s3:::photos/*"}"#;
    assert!(policy(&format!("{named},{}", allow_anyone(""))).is_public());
}

#[test]
fn a_fixed_source_address_or_user_name_narrows_everyone() {
    let narrow = [
        r#","Condition":{"IpAddress":{"aws:SourceIp":["10.0.0.0/8","192.168.1.0/24"]}}"#,
        r#","Condition":{"IpAddress":{"aws:SourceIp":"2001:db8::/32"}}"#,
        r#","Condition":{"StringEquals":{"aws:username":"ann"}}"#,
        r#","Condition":{"Bool":{"aws:SecureTransport":"true"},"IpAddress":{"aws:SourceIp":"10.1.2.3"}}"#,
    ];
    for condition in narrow {
        assert!(!policy(&allow_anyone(condition)).is_public(), "{condition}");
    }
}

#[test]
fn a_condition_that_does_not_fix_who_may_ask_leaves_it_public() {
    let wide = [
        // Wider than a /8, or every address there is.
        r#","Condition":{"IpAddress":{"aws:SourceIp":["10.0.0.0/8","0.0.0.0/0"]}}"#,
        r#","Condition":{"IpAddress":{"aws:SourceIp":"2001:db8::/16"}}"#,
        // Satisfied by a request that carries no address at all, or by every address but one.
        r#","Condition":{"IpAddressIfExists":{"aws:SourceIp":"10.0.0.0/8"}}"#,
        r#","Condition":{"NotIpAddress":{"aws:SourceIp":"10.0.0.0/8"}}"#,
        // Patterns, and keys that say nothing about who asks.
        r#","Condition":{"StringLike":{"aws:username":"a*"}}"#,
        r#","Condition":{"StringEquals":{"aws:username":"a*"}}"#,
        r#","Condition":{"Bool":{"aws:SecureTransport":"true"}}"#,
        r#","Condition":{"StringEquals":{"s3:prefix":"public/"}}"#,
    ];
    for condition in wide {
        assert!(policy(&allow_anyone(condition)).is_public(), "{condition}");
    }
}

#[test]
fn every_switch_is_on_until_a_configuration_says_otherwise() {
    let block = PublicAccessBlock::default();
    assert!(
        block.block_public_acls
            && block.ignore_public_acls
            && block.block_public_policy
            && block.restrict_public_buckets
    );
    let none = PublicAccessBlock::NONE;
    assert!(
        !(none.block_public_acls
            || none.ignore_public_acls
            || none.block_public_policy
            || none.restrict_public_buckets)
    );
}
