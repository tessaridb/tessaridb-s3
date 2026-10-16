//! The node's metrics in Prometheus text: every S3 request counted by operation, status and error code, its time to
//! the response by operation and size class, and the requests shed under load. Labels come only from bounded sets —
//! the operation catalog, HTTP statuses, the error catalog, five size classes — never from a bucket or a key.

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;

use crate::{Error, Result};

/// Size classes by their upper bound in bytes; a request at or over the last bound is `ge_128MiB`.
const SIZE_CLASSES: [(u64, &str); 4] = [
    (64 << 10, "lt_64KiB"),
    (1 << 20, "lt_1MiB"),
    (16 << 20, "lt_16MiB"),
    (128 << 20, "lt_128MiB"),
];

/// The size class of a request that moved `bytes` (the larger of its body and its response's).
#[must_use]
pub fn size_class(bytes: u64) -> &'static str {
    SIZE_CLASSES
        .iter()
        .find(|(bound, _)| bytes < *bound)
        .map_or("ge_128MiB", |(_, name)| name)
}

/// What one served request was.
#[derive(Debug, Clone, Copy)]
pub struct Served {
    /// Its operation's model name (`PutObject`), or `Unknown` before dispatch named one.
    pub operation: &'static str,
    /// The HTTP status answered.
    pub status: u16,
    /// The S3 error code, or `None` for a success.
    pub code: &'static str,
    /// The larger of the request body and the response body, in bytes.
    pub bytes: u64,
    /// Seconds from the request's arrival to its response.
    pub seconds: f64,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RequestLabels {
    operation: &'static str,
    status: u16,
    code: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct DurationLabels {
    operation: &'static str,
    size: &'static str,
}

/// A duration histogram from 1 ms to about 33 s.
fn duration_histogram() -> Histogram {
    Histogram::new(exponential_buckets(0.001, 2.0, 16))
}

/// The node's metrics.
#[derive(Debug)]
pub struct Metrics {
    registry: Registry,
    requests: Family<RequestLabels, Counter>,
    durations: Family<DurationLabels, Histogram, fn() -> Histogram>,
    shed: Counter,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Empty metrics.
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Registry::with_prefix("s3");
        let requests = Family::<RequestLabels, Counter>::default();
        let durations =
            Family::<DurationLabels, Histogram, fn() -> Histogram>::new_with_constructor(
                duration_histogram,
            );
        let shed = Counter::default();
        registry.register(
            "requests",
            "S3 requests by operation, status and error code",
            requests.clone(),
        );
        registry.register(
            "request_duration_seconds",
            "Time from a request's arrival to its response, by operation and size class",
            durations.clone(),
        );
        registry.register(
            "requests_shed",
            "Requests refused before the pipeline because the node was at its in-flight limit",
            shed.clone(),
        );
        Self {
            registry,
            requests,
            durations,
            shed,
        }
    }

    /// Counts one served request.
    pub fn served(&self, served: &Served) {
        self.requests
            .get_or_create(&RequestLabels {
                operation: served.operation,
                status: served.status,
                code: served.code,
            })
            .inc();
        self.durations
            .get_or_create(&DurationLabels {
                operation: served.operation,
                size: size_class(served.bytes),
            })
            .observe(served.seconds);
    }

    /// Counts one request shed for load.
    pub fn shed(&self) {
        self.shed.inc();
    }

    /// The metrics in Prometheus text.
    ///
    /// # Errors
    /// [`Error::Encode`] when the text could not be written.
    pub fn encode(&self) -> Result<String> {
        let mut text = String::new();
        encode(&mut text, &self.registry).map_err(|_| Error::Encode)?;
        Ok(text)
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
