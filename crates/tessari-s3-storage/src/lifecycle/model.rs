//! What one lifecycle pass did.

/// Counts of one pass, for the log and for tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Applied {
    /// Buckets whose rules were applied.
    pub buckets: usize,
    /// Current versions expired (a delete marker added, or the object removed where versions are not kept).
    pub expired: usize,
    /// Noncurrent versions removed.
    pub noncurrent: usize,
    /// Delete markers removed that were their key's only version.
    pub markers: usize,
    /// Multipart uploads aborted.
    pub aborted: usize,
}

impl Applied {
    /// Every action the pass took.
    #[must_use]
    pub const fn actions(&self) -> usize {
        self.expired
            .saturating_add(self.noncurrent)
            .saturating_add(self.markers)
            .saturating_add(self.aborted)
    }
}
