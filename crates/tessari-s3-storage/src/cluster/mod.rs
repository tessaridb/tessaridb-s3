//! The `cluster` domain: which S3 nodes exist and which layout their shards follow, held in TessariDB like every
//! other piece of metadata.

mod entity;
#[cfg(test)]
mod fake;
mod layouts;
mod repository;
mod service;
mod staging;
mod tessaridb;

pub use service::{ClusterService, Member};
pub use staging::{Apply, Stage, StagedLayout};
pub use tessaridb::TessariCluster;
