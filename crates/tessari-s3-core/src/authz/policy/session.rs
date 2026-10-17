//! A session policy: the IAM identity policy temporary credentials were issued with. It names no principal — it is
//! about the session — and may cover any S3 resource; a request the session's user may make stands only where the
//! policy also allows it, so a session never reaches further than its user, and a Deny in it outweighs everything.

use tessari_s3_constants::access::SESSION_POLICY_MAX_CHARS;

use super::document::{Governs, PolicyError, Statement, refuse, statements_of};
use super::evaluate::{PolicyAnswer, PolicyRequest, answer};

/// A session's policy, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPolicy {
    statements: Vec<Statement>,
}

impl SessionPolicy {
    /// The session policy `text`.
    ///
    /// # Errors
    /// [`PolicyError`] naming the first thing that is malformed or not evaluable here.
    pub fn parse(text: &str) -> Result<Self, PolicyError> {
        if text.chars().count() > SESSION_POLICY_MAX_CHARS {
            return Err(refuse(format!(
                "a session policy is at most {SESSION_POLICY_MAX_CHARS} characters"
            )));
        }
        Ok(Self {
            statements: statements_of(text, &Governs::Session)?,
        })
    }

    /// What this policy says about `request` on bucket `bucket` (none for an action that names no bucket, which is
    /// asked as `arn:aws:s3:::*`).
    #[must_use]
    pub fn evaluate(&self, request: &PolicyRequest<'_>, bucket: Option<&str>) -> PolicyAnswer {
        let resource = match (bucket, request.key) {
            (Some(bucket), Some(key)) => format!("arn:aws:s3:::{bucket}/{key}"),
            (Some(bucket), None) => format!("arn:aws:s3:::{bucket}"),
            (None, _) => "arn:aws:s3:::*".to_owned(),
        };
        answer(&self.statements, request, &resource, |_| true)
    }
}
