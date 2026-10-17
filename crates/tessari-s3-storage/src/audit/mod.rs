//! The `audit` domain: one record per S3 request in the region's `s3_access_log` topic (S3 ref 11 M6), each named by
//! its request id, so an append that is retried after an ambiguous failure is refused rather than doubled.

mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::AccessRecord;
pub use service::AuditService;
pub use tessaridb::TessariAudit;
