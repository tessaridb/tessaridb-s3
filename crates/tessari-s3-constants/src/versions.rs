//! The attempts of a write of a key that may lose to other writers of the same key.

/// Most attempts of one write or delete of a key that keeps losing to other writers of the same key; each attempt reads
/// the key's newest version again (ADR-0010). An attempt can lose to the same commit more than once while that commit
/// is not yet readable, so attempts are not one per other writer; the deadline normally ends a write before this does.
/// Past either the request is answered as contended (503).
pub const VERSION_WRITE_ATTEMPTS: u32 = 256;

/// How long one write of a key may keep losing to other writers before it is answered as contended.
pub const VERSION_WRITE_DEADLINE_MS: u64 = 5_000;

/// The first wait before a lost write of a key is attempted again, in milliseconds; each later wait doubles to
/// [`VERSION_WRITE_BACKOFF_MAX_MS`], and each is drawn between half of it and all of it.
pub const VERSION_WRITE_BACKOFF_FIRST_MS: u64 = 1;

/// The longest wait between two attempts of one write of a key, in milliseconds.
pub const VERSION_WRITE_BACKOFF_MAX_MS: u64 = 32;
