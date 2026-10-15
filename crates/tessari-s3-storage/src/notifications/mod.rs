//! The `notifications` domain: the events the store's own events append to the `object_events` topic, handed to
//! each operator-declared destination through its own topic group (S3 ref 11 M5) — at least once, with a deadline, a
//! bounded number of hand-outs and a dead letter.

mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::{Due, Event};
pub use service::NotificationService;
pub use tessaridb::TessariNotifications;
