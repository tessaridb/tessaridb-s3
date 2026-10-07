//! AWS Signature Version 4, verified server-side: the `Authorization` header, presigned URLs, the aws-chunked
//! signature chain, and the declared payload hash. Pure computation: the caller supplies the request as it
//! arrived, the secret for its access key, the server's region and the clock.

mod canonical;
mod chunk;
mod encode;
mod error;
mod header;
mod payload;
mod presign;
mod signing;
mod time;
mod verified;

pub use canonical::SignedRequest;
pub use chunk::{ChunkChain, parse_chunk_header};
pub use error::{AuthError, AuthResult};
pub use header::{AuthorizationHeader, verify_header};
pub use payload::{PayloadVerifier, parse_payload_hash};
pub use presign::{PresignedQuery, verify_presigned};
pub use signing::SecretKey;
pub use verified::{PayloadHash, Verified};
