//! Who may do what (ADR-0007): spaces own buckets, users belong to one space with a role, per-bucket grants narrow a
//! member, and two store-wide roles reach past spaces. One pure evaluator answers every enforcement point — the S3
//! router, the console API — so no handler carries permission logic of its own.

mod decide;
mod model;
mod operations;
pub mod policy;

pub use decide::{Asked, Decision, Denied, authorize, authorize_asked};
pub use model::{
    Access, Action, BucketResource, Principal, Role, SpaceName, UploadPrincipal, UserName,
    UserPrincipal, UserResource, Visible,
};
pub use operations::{Need, Required, required, required_naming_version};

#[cfg(test)]
mod asked_tests;
#[cfg(test)]
mod decide_tests;
#[cfg(test)]
mod operations_tests;
#[cfg(test)]
mod upload_tests;
