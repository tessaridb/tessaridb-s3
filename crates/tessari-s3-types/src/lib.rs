//! Values shared across crates: bucket names, object keys, the secret-key wrapper, the S3 error codes and the error
//! taxonomy.

mod bucket_name;
mod category;
mod error;
mod error_code;
mod object_key;
mod secret;

pub use bucket_name::BucketName;
pub use category::ErrorCategory;
pub use error::{Error, Result};
pub use error_code::ErrorCode;
pub use object_key::ObjectKey;
pub use secret::SecretKey;
