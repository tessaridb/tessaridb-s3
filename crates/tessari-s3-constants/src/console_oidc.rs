//! The console's client of an OpenID Connect provider: what a sign-in may carry and how long its pieces live.

/// The `amr` value (RFC 8176 "multiple-factor authentication") a console sign-in through an identity provider must
/// carry when the operator names no other.
pub const CONSOLE_OIDC_DEFAULT_MFA_AMR: &str = "mfa";

/// How long the console's client assertion (`private_key_jwt`) is valid after it is signed; it is used at once.
pub const CONSOLE_OIDC_ASSERTION_SECS: i64 = 60;

/// How long a console sign-in sent to the identity provider waits for its callback before the store drops it.
pub const CONSOLE_OIDC_LOGIN_SECS: i64 = 600;

/// How old a back-channel logout token may be when it arrives, and how long its id is kept so it is acted on once.
pub const CONSOLE_OIDC_LOGOUT_MAX_AGE_SECS: i64 = 300;

/// How far ahead of this node's clock a logout token's issue time may be.
pub const CONSOLE_OIDC_LOGOUT_AHEAD_SECS: i64 = 60;

/// Least time between two fetches of the identity provider's discovery document and keys, however many ID tokens
/// name a key the console does not hold — a flood of invented key ids cannot turn into a flood against the provider.
pub const CONSOLE_OIDC_REFRESH_MIN_SECS: i64 = 60;

/// How long one request to the identity provider (discovery, keys, the code exchange) may take.
pub const CONSOLE_OIDC_HTTP_TIMEOUT_SECS: u64 = 10;

/// Largest answer the console reads from the identity provider; a longer one is refused, not buffered.
pub const CONSOLE_OIDC_RESPONSE_MAX_BYTES: usize = 1 << 20;
