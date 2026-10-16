//! Opening the encryption keys vault.

/// How many times opening the keys vault is tried while it loses to other catalog commits or to another node's seal.
pub const KEYS_OPEN_ATTEMPTS: u32 = 16;

/// The first wait before the keys vault is opened again, in milliseconds, growing per attempt.
pub const KEYS_OPEN_BACKOFF_MS: u64 = 20;
