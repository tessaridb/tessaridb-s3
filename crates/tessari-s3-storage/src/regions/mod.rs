//! The `regions` domain: which regions an operator failed over to another, under which epoch. A failed-over region
//! refuses client writes until it is restored; every failover and restore moves the region's epoch on by one.

mod entity;
mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::{FailOver, RegionState, Restore};
pub use service::RegionService;
pub use tessaridb::TessariRegions;
