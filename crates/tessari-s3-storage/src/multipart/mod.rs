//! The `multipart` domain: uploads and their parts in TessariDB, each part's bytes in its own data file.

mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::{Created, NewPart, NewUpload, PartWritten, PartsPage, StoredPart};
pub use service::MultipartService;
pub use tessaridb::TessariMultipart;
