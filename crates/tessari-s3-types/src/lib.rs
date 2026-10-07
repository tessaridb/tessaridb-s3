//! Validated values shared across crates: bucket names, object keys, and the error taxonomy.

mod bucket_name;
mod category;
mod error;
mod object_key;

pub use bucket_name::BucketName;
pub use category::ErrorCategory;
pub use error::{Error, Result};
pub use object_key::ObjectKey;
