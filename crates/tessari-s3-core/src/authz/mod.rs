//! Who may do what (ADR-0007): spaces own buckets, users belong to one space with a role, per-bucket grants narrow a
//! member, and two store-wide roles reach past spaces. One pure evaluator answers every enforcement point — the S3
//! router, the console API — so no handler carries permission logic of its own.

mod decide;
mod model;
mod operations;

pub use decide::{Decision, Denied, authorize};
pub use model::{
    Access, Action, BucketResource, Principal, Role, SpaceName, UserPrincipal, Visible,
};
pub use operations::{Need, Required, required};

#[cfg(test)]
mod decide_tests;
#[cfg(test)]
mod operations_tests;
