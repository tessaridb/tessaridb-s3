//! What one phase measured: a latency histogram, refusals by code, correctness mismatches, client preparation time.

use std::collections::BTreeMap;
use std::time::Duration;

use hdrhistogram::Histogram;
use tessari_s3_constants::{BENCH_HISTOGRAM_DIGITS, BENCH_HISTOGRAM_MAX_MICROS};

use crate::client::Failure;
use crate::error::Result;

/// The result of one operation.
#[derive(Debug)]
pub struct OpOutcome {
    /// Time on the wire: from sending the request to the last byte of the answer (for multipart, every request).
    pub latency: Duration,
    /// Client time spent before the timer started (building and hashing the body).
    pub prep: Duration,
    /// Whether the server did what was asked.
    pub result: std::result::Result<(), Failure>,
    /// A GET whose body is not what was PUT.
    pub mismatch: bool,
}

/// One phase's measurements, recorded by the loop that collects every operation's outcome.
#[derive(Debug, Clone)]
pub struct Recorder {
    /// Latency of every successful measured operation, in microseconds.
    pub histogram: Histogram<u64>,
    /// Failed operations by label.
    pub failures: BTreeMap<String, u64>,
    /// GET bodies that did not match.
    pub mismatches: u64,
    /// Client preparation time over the measured operations.
    pub prep: Duration,
}

impl Recorder {
    /// An empty recorder.
    ///
    /// # Errors
    /// [`crate::Error::Histogram`] if the configured bounds are rejected.
    pub fn new() -> Result<Self> {
        Ok(Self {
            histogram: Histogram::new_with_bounds(
                1,
                BENCH_HISTOGRAM_MAX_MICROS,
                BENCH_HISTOGRAM_DIGITS,
            )?,
            failures: BTreeMap::new(),
            mismatches: 0,
            prep: Duration::ZERO,
        })
    }

    /// Counts one measured operation.
    pub fn record(&mut self, outcome: &OpOutcome) {
        self.prep = self.prep.saturating_add(outcome.prep);
        if outcome.mismatch {
            self.mismatches = self.mismatches.saturating_add(1);
        }
        match &outcome.result {
            Ok(()) => {
                let micros = u64::try_from(outcome.latency.as_micros()).unwrap_or(u64::MAX);
                self.histogram.saturating_record(micros.max(1));
            }
            Err(failure) => {
                let count = self.failures.entry(failure.label()).or_default();
                *count = count.saturating_add(1);
            }
        }
    }
}
