//! The `objects` domain: object records in TessariDB, every object's bytes in data files (ADR-0008).

mod entity;
mod model;
mod model_lock;
mod model_tags;
mod reader;
mod repository;
mod service;
mod tessaridb;
mod upload;
pub(crate) mod version_id;

pub use model::{
    Content, Deleted, HealBacklog, Healed, Listed, ListedVersion, MoveBacklog, Moved, Multipart,
    NewObject, ObjectSummary, Part, Reclaimed, Removed, RemovedIf, Replicated, StoredObject,
    VersionKind, VersionRead, VersionRemoved, VersionsListed, WriteCondition, Written,
};
pub use model_lock::{Lock, LockOutcome, RetainUntil, VersionLock};
pub use model_tags::TagsOutcome;
pub use reader::ObjectReader;
pub use service::ObjectService;
pub use tessaridb::TessariObjects;
pub use upload::{Upload, Uploaded};
