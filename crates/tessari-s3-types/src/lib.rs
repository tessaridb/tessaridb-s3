//! Values shared across crates: bucket names, object keys, the secret-key wrapper, the S3 error codes and the error
//! taxonomy.

mod bucket_name;
mod category;
mod erasure_code;
mod error;
mod error_code;
mod iam_key;
mod multipart;
mod node_id;
mod object_key;
mod secret;
mod timestamp;

pub use bucket_name::BucketName;
pub use category::ErrorCategory;
pub use erasure_code::{Code, CodeError};
pub use error::{Error, Result};
pub use error_code::ErrorCode;
pub use iam_key::IamKey;
pub use multipart::{PartNumber, UploadId};
pub use node_id::NodeId;
pub use object_key::ObjectKey;
pub use secret::SecretKey;
pub use timestamp::Timestamp;
