//! The `cluster` domain: which S3 nodes exist and which layout their shards follow, held in TessariDB like every
//! other piece of metadata.

mod entity;
mod repository;
mod service;
mod tessaridb;

pub use service::{ClusterService, Member};
pub use tessaridb::TessariCluster;
