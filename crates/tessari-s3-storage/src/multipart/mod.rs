//! The `multipart` domain: uploads and their parts in TessariDB, each part's bytes in its own data file.

mod entity;
mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::{
    Completed, Completion, Created, NewPart, NewUpload, PartWritten, PartsPage, StoredPart,
};
pub use service::MultipartService;
pub use tessaridb::TessariMultipart;
