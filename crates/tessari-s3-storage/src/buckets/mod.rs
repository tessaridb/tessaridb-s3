//! The `buckets` domain: bucket records in TessariDB.

mod entity;
mod lifecycle;
mod media;
mod model;
mod notification;
pub(crate) mod replication;
mod repository;
mod service;
mod tessaridb;

pub(crate) use model::LOCAL;
pub use model::{Acknowledgement, Bucket};
pub use service::{AcknowledgementSet, BucketService, Created, Deleted};
pub use tessaridb::TessariBuckets;
