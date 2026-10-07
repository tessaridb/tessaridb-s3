//! The `buckets` domain: bucket records in TessariDB.

mod entity;
mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::Bucket;
pub use service::{BucketService, Created, Deleted};
pub use tessaridb::TessariBuckets;
