//! What an operation handler receives: the request resolved, decoded and authenticated.

use axum::http::HeaderMap;
use tessari_s3_core::auth::Verified;
use tessari_s3_core::authz::Principal;
use tessari_s3_core::authz::policy::Context;
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
    /// Who is asking; the operation was already authorized for it.
    pub principal: &'a Principal,
    /// The request's headers.
    pub headers: &'a HeaderMap,
    /// Whether it arrived over a connection whose TLS handshake completed.
    pub secure: bool,
    /// What a bucket policy's conditions read of this request.
    pub context: &'a Context,
}

impl Call<'_> {
    /// The owner every answer names: the space — the account a bucket and everything in it belong to, and the only
    /// one a principal reaches — so a rotated key or a granted reader never shows up as one; root, whose buckets are
    /// in every space, is named by its key.
    #[must_use]
    pub fn owner(&self) -> String {
        fn space(principal: &Principal) -> Option<&str> {
            match principal {
                Principal::User(user) => Some(user.space.as_str()),
                Principal::Upload(upload) => Some(upload.parent.space.as_str()),
                Principal::Session(session) => space(session.as_user()),
                Principal::Root | Principal::Anonymous => None,
            }
        }
        space(self.principal)
            .unwrap_or_else(|| self.verified.access_key())
            .to_owned()
    }

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
