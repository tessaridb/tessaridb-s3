//! The `lifecycle` domain: the engine that applies buckets' lifecycle rules (S3 ref 08 M7), driven by the
//! `lifecycle_runs` queue in TessariDB so one node at a time works a bucket.

mod entity;
mod model;
mod repository;
mod rules;
mod service;
mod service_walk;
mod tessaridb;

pub use model::Applied;
pub use service::LifecycleService;
pub use tessaridb::TessariLifecycle;
