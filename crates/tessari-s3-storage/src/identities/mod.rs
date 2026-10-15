//! The `identities` domain: console sign-in through an identity provider — the in-flight logins awaiting their
//! callback, and which S3 user each provider identity is linked to.

mod entity;
mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::{Linked, PendingLogin};
pub use service::IdentityService;
pub use tessaridb::TessariIdentities;
