//! AWS Signature Version 4, verified server-side: the `Authorization` header, presigned URLs, the aws-chunked
//! signature chain, and the declared payload hash. Pure computation: the caller supplies the request as it
//! arrived, the secret for its access key, the server's region and the clock.

mod canonical;
mod chunk;
mod chunked;
mod encode;
mod error;
mod header;
mod payload;
mod post_policy;
#[cfg(test)]
mod post_policy_tests;
mod presign;
mod sign;
mod signing;
mod time;
mod verified;

pub use canonical::SignedRequest;
pub use chunk::{ChunkChain, parse_chunk_header};
pub use chunked::{ChunkedDecoder, Trailer};
pub use encode::{percent_decode, uri_encode};
pub use error::{AuthError, AuthResult};
pub use header::{AuthorizationHeader, verify_header};
pub use payload::{PayloadVerifier, parse_payload_hash, verify_payload_digest};
pub use post_policy::{PostPolicy, PostPolicyError, post_signature, verify_post};
pub use presign::{PresignedQuery, verify_presigned};
pub use sign::{amz_date, sign_header};
pub use tessari_s3_types::SecretKey;
pub use verified::{PayloadHash, Verified};
