//! What an operation handler receives: the request resolved, decoded and authenticated.

use tessari_s3_core::auth::Verified;
use tessari_s3_types::{BucketName, ErrorCode};

use super::address::Addressed;
use crate::Error;
use crate::state::ApiState;

/// One authenticated request, ready for its operation.
pub struct Call<'a> {
    /// The shared server state.
    pub state: &'a ApiState,
    /// The bucket and key the path addressed.
    pub addressed: &'a Addressed,
    /// The decoded query parameters.
    pub query: &'a [(String, Option<String>)],
    /// Who signed it, and how its body is protected.
    pub verified: &'a Verified,
}

impl Call<'_> {
    /// The bucket a bucket or object operation addresses. Dispatch guarantees one; its absence is a defect.
    ///
    /// # Errors
    /// `InternalError` when the path addressed no bucket.
    pub fn bucket(&self) -> Result<&BucketName, Error> {
        self.addressed.bucket.as_ref().ok_or_else(|| {
            Error::new(
                ErrorCode::InternalError,
                "a bucket operation addressed no bucket",
            )
        })
    }

    /// The value of query parameter `name`, if present with a value.
    #[must_use]
    pub fn query_value(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, value)| value.as_deref())
    }
}
