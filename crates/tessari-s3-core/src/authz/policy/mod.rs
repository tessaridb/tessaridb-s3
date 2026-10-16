//! Bucket policies (S3 ref 04): the JSON document a bucket carries, validated when it is written, and the pure
//! evaluation the one authorizer runs on every path — deny overrides allow, and nothing applies unless the
//! principal, the action, the resource and every condition match.

mod actions;
mod condition;
mod context;
mod document;
mod evaluate;
mod public;
mod wildcard;

pub use actions::{S3_ACTIONS, Scope, policy_action};
pub use context::{Context, Requester};
pub use document::{BucketPolicy, Effect, PolicyError, Statement};
pub use evaluate::{PolicyAnswer, PolicyRequest};
pub use public::PublicAccessBlock;

#[cfg(test)]
mod actions_tests;
#[cfg(test)]
mod document_tests;
#[cfg(test)]
mod evaluate_tests;
#[cfg(test)]
mod public_tests;
