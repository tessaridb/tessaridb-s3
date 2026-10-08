//! The `objects` domain: object records in TessariDB, small objects inline and larger ones in data files.

mod entity;
mod model;
mod reader;
mod repository;
mod service;
mod tessaridb;
mod upload;

pub use model::{
    Content, HealBacklog, Healed, Listed, Multipart, NewObject, ObjectSummary, Part, Reclaimed,
    Removed, RemovedIf, StoredObject, WriteCondition, Written,
};
pub use reader::ObjectReader;
pub use service::ObjectService;
pub use tessaridb::TessariObjects;
pub use upload::{Upload, Uploaded};
