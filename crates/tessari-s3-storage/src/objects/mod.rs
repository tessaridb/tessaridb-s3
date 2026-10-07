//! The `objects` domain: object records in TessariDB, small objects inline.

mod entity;
mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::{NewObject, Removed, StoredObject, WriteCondition, Written};
pub use service::ObjectService;
pub use tessaridb::TessariObjects;
