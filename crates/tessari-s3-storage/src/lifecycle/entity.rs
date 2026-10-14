//! A `lifecycle_runs` record exactly as stored.

/// One bucket's place in the lifecycle queue, as a claim handed it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunEntity {
    /// The bucket whose rules are due.
    pub bucket: String,
    /// When it became due: seconds and nanoseconds since the epoch. Rewritten whenever the rules are, so a claim can
    /// tell whether they changed while it held the record.
    pub due: (i64, u32),
}
