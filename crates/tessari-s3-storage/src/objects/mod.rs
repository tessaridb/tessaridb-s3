//! The `objects` domain: object records in TessariDB, every object's bytes in data files (ADR-0008).

mod entity;
mod model;
mod reader;
mod repository;
mod service;
mod tessaridb;
mod upload;
mod version_id;

pub use model::{
    Content, Deleted, HealBacklog, Healed, Listed, Multipart, NewObject, ObjectSummary, Part,
    Reclaimed, Removed, RemovedIf, StoredObject, VersionRead, VersionRemoved, WriteCondition,
    Written,
};
pub use reader::ObjectReader;
pub use service::ObjectService;
pub use tessaridb::TessariObjects;
pub use upload::{Upload, Uploaded};
