//! The node's metrics in Prometheus text: every S3 request counted by operation, status and error code, its time to
//! the response by operation and size class, the requests shed under load, authentication refusals by reason and
//! policy denials by operation; and, read when scraped, the heal and replication backlogs with whether each could be
//! read. Labels come only from bounded sets — the operation catalog, HTTP statuses, the error catalog, five size
//! classes, fixed reasons and sources — never from a bucket, a key or an access key.

use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;

use crate::metrics_labels::{
    AuthRefusal, DurationLabels, OperationLabels, ReasonLabels, RequestLabels, Served,
    SourceLabels, Unlabelled, gauge_value, size_class,
};
use crate::metrics_state::StateMetrics;
use crate::{Error, Result};

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
    auth_refusals: Family<ReasonLabels, Counter>,
    policy_denials: Family<OperationLabels, Counter>,
    heal_backlog: Family<Unlabelled, Gauge>,
    heal_at_risk: Family<Unlabelled, Gauge>,
    replication_backlog: Family<Unlabelled, Gauge>,
    replication_in_flight: Family<Unlabelled, Gauge>,
    replication_failed: Family<Unlabelled, Gauge>,
    members: Family<Unlabelled, Gauge>,
    members_answering: Family<Unlabelled, Gauge>,
    collect_up: Family<SourceLabels, Gauge>,
    audit_written: Counter,
    audit_dropped: Counter,
    audit_failed: Counter,
    verification_failures: Counter,
    state: StateMetrics,
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
        let auth_refusals = Family::<ReasonLabels, Counter>::default();
        registry.register(
            "auth_refusals",
            "Requests that did not authenticate, by reason",
            auth_refusals.clone(),
        );
        let policy_denials = Family::<OperationLabels, Counter>::default();
        registry.register(
            "policy_denials",
            "Authenticated requests the access policy denied, by operation",
            policy_denials.clone(),
        );
        let gauges = [
            (
                "heal_backlog",
                "Data ids listed for healing, counted up to a bound",
            ),
            (
                "heal_backlog_at_risk",
                "Of those counted, the ids handed out for healing repeatedly",
            ),
            ("replication_backlog", "Replication tasks not yet settled"),
            (
                "replication_in_flight",
                "Replication tasks handed out and not yet settled",
            ),
            (
                "replication_failed",
                "Replication tasks given up, their versions FAILED",
            ),
            (
                "cluster_members",
                "Nodes the cluster's metadata names as members",
            ),
            (
                "cluster_members_answering",
                "Of those members, the ones that answered a probe at this scrape",
            ),
        ]
        .map(|(name, help)| {
            let gauge = Family::<Unlabelled, Gauge>::default();
            registry.register(name, help, gauge.clone());
            gauge
        });
        let [
            heal_backlog,
            heal_at_risk,
            replication_backlog,
            replication_in_flight,
            replication_failed,
            members,
            members_answering,
        ] = gauges;
        let collect_up = Family::<SourceLabels, Gauge>::default();
        registry.register(
            "collect_up",
            "Whether the last scrape could read a source (1) or not (0); a source not read keeps its last values",
            collect_up.clone(),
        );
        let [audit_written, audit_dropped, audit_failed, verification_failures] = [
            ("audit_written", "Access-log records appended"),
            (
                "audit_dropped",
                "Access-log records not queued because the writer's queue was full; the requests were served",
            ),
            (
                "audit_failed",
                "Access-log records whose append failed after every retry",
            ),
            (
                "data_verification_failures",
                "Reads of this node's data files that failed verification (a block hash, a header or a length)",
            ),
        ]
        .map(|(name, help)| {
            let counter = Counter::default();
            registry.register(name, help, counter.clone());
            counter
        });
        let state = StateMetrics::register(&mut registry, env!("CARGO_PKG_VERSION"));
        Self {
            registry,
            requests,
            durations,
            shed,
            auth_refusals,
            policy_denials,
            heal_backlog,
            heal_at_risk,
            replication_backlog,
            replication_in_flight,
            replication_failed,
            members,
            members_answering,
            collect_up,
            audit_written,
            audit_dropped,
            audit_failed,
            verification_failures,
            state,
        }
    }

    /// The node's state and health series.
    #[must_use]
    pub const fn state(&self) -> &StateMetrics {
        &self.state
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

    /// Counts one request that did not authenticate.
    pub fn auth_refused(&self, refusal: AuthRefusal) {
        self.auth_refusals
            .get_or_create(&ReasonLabels {
                reason: refusal.label(),
            })
            .inc();
    }

    /// Counts one authenticated request the access policy denied.
    pub fn denied(&self, operation: &'static str) {
        self.policy_denials
            .get_or_create(&OperationLabels { operation })
            .inc();
    }

    /// Sets the heal backlog as last read: ids listed and of them at risk.
    pub fn heal(&self, listed: u64, at_risk: u64) {
        self.heal_backlog
            .get_or_create(&Unlabelled)
            .set(gauge_value(listed));
        self.heal_at_risk
            .get_or_create(&Unlabelled)
            .set(gauge_value(at_risk));
    }

    /// Sets the replication backlog as last read.
    pub fn replication(&self, backlog: u64, in_flight: u64, failed: u64) {
        self.replication_backlog
            .get_or_create(&Unlabelled)
            .set(gauge_value(backlog));
        self.replication_in_flight
            .get_or_create(&Unlabelled)
            .set(gauge_value(in_flight));
        self.replication_failed
            .get_or_create(&Unlabelled)
            .set(gauge_value(failed));
    }

    /// Sets the cluster's member count and how many of them answered.
    pub fn cluster(&self, members: u64, answering: u64) {
        self.members
            .get_or_create(&Unlabelled)
            .set(gauge_value(members));
        self.members_answering
            .get_or_create(&Unlabelled)
            .set(gauge_value(answering));
    }

    /// Records whether `source` (`heal`, `replication`, `cluster`) could be read at this scrape.
    pub fn collected(&self, source: &'static str, read: bool) {
        self.collect_up
            .get_or_create(&SourceLabels { source })
            .set(i64::from(read));
    }

    /// Counts `records` appended to the access log.
    pub fn audit_written(&self, records: u64) {
        self.audit_written.inc_by(records);
    }

    /// Counts one access-log record the full queue turned away.
    pub fn audit_dropped(&self) {
        self.audit_dropped.inc();
    }

    /// Counts `records` whose append failed for good.
    pub fn audit_failed(&self, records: u64) {
        self.audit_failed.inc_by(records);
    }

    /// Raises the count of failed verifications to `total`, as the storage last counted it since the node started.
    pub fn verification_failures(&self, total: u64) {
        self.verification_failures
            .inc_by(total.saturating_sub(self.verification_failures.get()));
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
