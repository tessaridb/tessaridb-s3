//! The `replication` domain (S3 ref 11 M4): the tasks the store's own `replication_queued` event appends to the
//! `replication_tasks` topic, handed to this region's replicators through one topic group — at least once, with a
//! deadline, a bounded number of hand-outs and a dead letter — the status a version is stamped with once sent or
//! given up, and the URL each region's S3 surface is reached at.

mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::{Progress, Task};
pub use service::ReplicationService;
pub use tessaridb::TessariReplication;
