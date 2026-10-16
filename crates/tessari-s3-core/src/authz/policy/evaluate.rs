//! Evaluating a bucket policy for one request: any matching Deny ends it, else any matching Allow answers, else the
//! policy says nothing — and the caller's own grants decide.

use super::context::{Context, Requester};
use super::document::{BucketPolicy, Effect, Patterns, Principals, Statement};
use super::wildcard;

/// One question asked of a policy.
#[derive(Debug, Clone, Copy)]
pub struct PolicyRequest<'a> {
    /// Who asks.
    pub requester: Requester<'a>,
    /// The S3 action, `s3:GetObject`.
    pub action: &'a str,
    /// The object key; `None` asks about the bucket itself.
    pub key: Option<&'a str>,
    /// The request's facts for conditions.
    pub context: &'a Context,
}

/// A policy's answer, naming the statement that gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyAnswer {
    /// A Deny matched: refused, whatever else allows it.
    Deny {
        /// The statement's Sid.
        sid: Option<String>,
    },
    /// An Allow matched and no Deny did.
    Allow {
        /// The statement's Sid.
        sid: Option<String>,
    },
    /// No statement matched.
    NotApplicable,
}

impl BucketPolicy {
    /// What this policy says about `request`.
    #[must_use]
    pub fn evaluate(&self, request: &PolicyRequest<'_>) -> PolicyAnswer {
        let resource = match request.key {
            Some(key) => format!("arn:aws:s3:::{}/{key}", self.bucket),
            None => format!("arn:aws:s3:::{}", self.bucket),
        };
        let mut allowed = None;
        for statement in self
            .statements
            .iter()
            .filter(|s| self.applies(s, request, &resource))
        {
            match statement.effect {
                Effect::Deny => {
                    return PolicyAnswer::Deny {
                        sid: statement.sid.clone(),
                    };
                }
                Effect::Allow => {
                    allowed.get_or_insert_with(|| statement.sid.clone());
                }
            }
        }
        allowed.map_or(PolicyAnswer::NotApplicable, |sid| PolicyAnswer::Allow {
            sid,
        })
    }

    fn applies(&self, statement: &Statement, request: &PolicyRequest<'_>, resource: &str) -> bool {
        self.names(&statement.principals, request.requester)
            && listed(&statement.actions, |pattern| {
                wildcard::matches_ignoring_case(pattern, request.action)
            })
            && listed(&statement.resources, |pattern| {
                wildcard::matches(pattern, resource)
            })
            && statement
                .conditions
                .iter()
                .all(|condition| condition.holds(request.requester, request.context))
    }

    fn names(&self, principals: &Principals, requester: Requester<'_>) -> bool {
        match (principals, requester) {
            (Principals::Anyone, _) => true,
            (Principals::Named { whole_space, users }, Requester::User { space, name }) => {
                *space == self.space && (*whole_space || users.iter().any(|user| user == name))
            }
            (Principals::Named { .. }, Requester::Anonymous) => false,
        }
    }
}

/// Whether the patterns cover the value: any of them, or — for a `Not…` list — none of them.
fn listed(patterns: &Patterns, matches: impl Fn(&str) -> bool) -> bool {
    patterns.list.iter().any(|pattern| matches(pattern)) != patterns.negated
}
