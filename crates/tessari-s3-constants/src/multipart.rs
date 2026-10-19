//! Multipart commits.

/// Times a part's commit is run while the store answers `retry`: every part of one upload writes the upload's
/// record, so parts a client sends at once (the AWS CLI sends ten) meet each other; a refused commit wrote nothing.
pub const PART_COMMIT_ATTEMPTS: u32 = 16;

/// Milliseconds of back-off per attempt between part commits (10, 20, 30 … ms, plus a spread).
pub const PART_COMMIT_BACKOFF_MS: u64 = 10;
