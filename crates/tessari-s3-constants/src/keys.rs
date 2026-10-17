//! Opening the encryption keys vault, and re-wrapping data keys after a rotation.

/// How many times opening the keys vault is tried while it loses to other catalog commits or to another node's seal.
pub const KEYS_OPEN_ATTEMPTS: u32 = 16;

/// The first wait before the keys vault is opened again, in milliseconds, growing per attempt.
pub const KEYS_OPEN_BACKOFF_MS: u64 = 20;

/// Most records one re-wrap pass looks at; the rest wait for the next pass.
pub const REWRAP_BATCH: usize = 500;

/// Seconds between re-wrap passes on a node.
pub const REWRAP_INTERVAL_SECS: u64 = 60;
