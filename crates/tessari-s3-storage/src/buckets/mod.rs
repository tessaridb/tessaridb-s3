//! The `buckets` domain: bucket records in TessariDB.

mod cors;
mod encryption;
mod entity;
mod lifecycle;
mod media;
mod model;
mod notification;
pub(crate) mod object_lock;
mod public_access;
pub(crate) mod replication;
mod repository;
mod service;
mod service_encryption;
mod tessaridb;

pub(crate) use model::LOCAL;
pub use model::{Acknowledgement, Bucket, BucketEncryption, LockSet};
pub use public_access::PublicAccess;
pub use service::{AcknowledgementSet, BucketService, Created, Deleted};
pub use tessaridb::TessariBuckets;
