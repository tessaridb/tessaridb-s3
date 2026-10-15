//! The `spaces` domain: the tenants buckets and users belong to.

mod entity;
mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::Space;
pub use service::{SpaceCreated, SpaceService};
pub use tessaridb::TessariSpaces;
