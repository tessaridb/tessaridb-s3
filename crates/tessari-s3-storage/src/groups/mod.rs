//! The `groups` domain: named sets of a space's users, each carrying per-bucket grants its members receive as if they
//! were their own.

mod entity;
mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::{Group, GroupGrant, MemberChange};
pub use service::GroupService;
pub use tessaridb::TessariGroups;
