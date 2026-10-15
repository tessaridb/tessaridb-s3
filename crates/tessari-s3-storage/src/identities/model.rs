//! What callers hand the identities domain and what it answers.

use tessari_s3_types::SecretKey;

/// A login sent to the identity provider and not yet returned: what its callback must match.
#[derive(Debug, Clone)]
pub struct PendingLogin {
    /// The `state` the provider hands back; the login is found by it, once.
    pub state: String,
    /// The `nonce` the ID token must carry.
    pub nonce: SecretKey,
    /// The PKCE verifier the code is exchanged with.
    pub verifier: SecretKey,
}

/// What linking a user to an identity ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Linked {
    /// The identity now signs in as the user; any earlier link of that user is gone.
    Linked,
    /// Another user holds the identity; nothing changed.
    TakenByAnother,
}
