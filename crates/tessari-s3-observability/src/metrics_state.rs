//! The node's state and health in its metrics: which build it runs and since when, how long the metadata store took to
//! answer a probe, whether the keys vault is open, how full the data directory's file system is, what the latest usage
//! measurement counted, how many uploads are open, and every daemon run by its outcome. Everything but the daemon runs
//! is read at the scrape; a gauge never read has no sample, never a zero.

use std::sync::atomic::AtomicU64;
use std::time::{SystemTime, UNIX_EPOCH};

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;

use crate::metrics_labels::{Unlabelled, gauge_value};

/// The keys vault's state as a scrape saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultState {
    /// Its keys are held: sealed objects can be read and written.
    Unsealed,
    /// It exists and is closed: every request needing one of its keys answers 503.
    Sealed,
    /// No vault was ever created on this store.
    Uninitialised,
}

impl VaultState {
    const ALL: [Self; 3] = [Self::Unsealed, Self::Sealed, Self::Uninitialised];

    const fn label(self) -> &'static str {
        match self {
            Self::Unsealed => "unsealed",
            Self::Sealed => "sealed",
            Self::Uninitialised => "uninitialised",
        }
    }
}

/// How one daemon run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// It reported success, idle or not.
    Succeeded,
    /// It returned an error.
    Failed,
    /// It was abandoned at its timeout.
    TimedOut,
}

impl RunOutcome {
    const fn label(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct VersionLabels {
    version: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct StateLabels {
    state: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RunLabels {
    daemon: &'static str,
    outcome: &'static str,
}

/// What a usage measurement counted, summed over its buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counted {
    /// Buckets measured.
    pub buckets: u64,
    /// Objects in them.
    pub objects: u64,
    /// Their bytes as clients wrote them.
    pub bytes: u64,
    /// When the measurement was taken, in unix seconds.
    pub taken_secs: i64,
}

/// The state and health series.
#[derive(Debug)]
pub struct StateMetrics {
    meta_probe: Family<Unlabelled, Gauge<f64, AtomicU64>>,
    vault: Family<StateLabels, Gauge>,
    capacity: Family<Unlabelled, Gauge>,
    free: Family<Unlabelled, Gauge>,
    buckets: Family<Unlabelled, Gauge>,
    objects: Family<Unlabelled, Gauge>,
    bytes: Family<Unlabelled, Gauge>,
    usage_taken: Family<Unlabelled, Gauge>,
    uploads: Family<Unlabelled, Gauge>,
    runs: Family<RunLabels, Counter>,
}

impl StateMetrics {
    /// Registers the series in `registry`; `version` is the build's, and the start time is now.
    pub(crate) fn register(registry: &mut Registry, version: &'static str) -> Self {
        let build = Family::<VersionLabels, Gauge>::default();
        build.get_or_create(&VersionLabels { version }).set(1);
        registry.register(
            "build_info",
            "The build this node runs, as its version label",
            build,
        );
        let started = Gauge::<i64>::default();
        started.set(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| gauge_value(since.as_secs())),
        );
        registry.register(
            "start_time_seconds",
            "When this node started, in unix seconds",
            started,
        );
        let meta_probe = Family::<Unlabelled, Gauge<f64, AtomicU64>>::default();
        registry.register(
            "meta_probe_seconds",
            "How long the metadata store took to answer a probe at the last scrape that reached it",
            meta_probe.clone(),
        );
        let vault = Family::<StateLabels, Gauge>::default();
        registry.register(
            "keys_vault",
            "The keys vault's state at the last scrape that read it: 1 for the current state, 0 for the others",
            vault.clone(),
        );
        let [
            capacity,
            free,
            buckets,
            objects,
            bytes,
            usage_taken,
            uploads,
        ] = [
            (
                "data_capacity_bytes",
                "Size of the file system holding the data directory",
            ),
            (
                "data_free_bytes",
                "Bytes of that file system this node may still write",
            ),
            (
                "buckets",
                "Buckets holding at least one object in the latest usage measurement",
            ),
            ("objects", "Objects in the latest usage measurement"),
            (
                "stored_bytes",
                "Object bytes, as written by clients, in the latest usage measurement",
            ),
            (
                "usage_taken_seconds",
                "When the latest usage measurement was taken, in unix seconds",
            ),
            (
                "uploads_open",
                "Multipart uploads neither completed nor aborted, counted up to a bound",
            ),
        ]
        .map(|(name, help)| {
            let gauge = Family::<Unlabelled, Gauge>::default();
            registry.register(name, help, gauge.clone());
            gauge
        });
        let runs = Family::<RunLabels, Counter>::default();
        registry.register(
            "daemon_runs",
            "Daemon runs by daemon and outcome",
            runs.clone(),
        );
        Self {
            meta_probe,
            vault,
            capacity,
            free,
            buckets,
            objects,
            bytes,
            usage_taken,
            uploads,
            runs,
        }
    }

    /// Sets how long the metadata store took to answer the probe.
    pub fn meta_probe(&self, seconds: f64) {
        self.meta_probe.get_or_create(&Unlabelled).set(seconds);
    }

    /// Sets the keys vault's state.
    pub fn vault(&self, current: VaultState) {
        for state in VaultState::ALL {
            self.vault
                .get_or_create(&StateLabels {
                    state: state.label(),
                })
                .set(i64::from(state == current));
        }
    }

    /// Sets the data file system's size and the bytes this node may still write there.
    pub fn data_space(&self, capacity: u64, free: u64) {
        self.capacity
            .get_or_create(&Unlabelled)
            .set(gauge_value(capacity));
        self.free.get_or_create(&Unlabelled).set(gauge_value(free));
    }

    /// Sets what the latest usage measurement counted.
    pub fn usage(&self, counted: Counted) {
        self.buckets
            .get_or_create(&Unlabelled)
            .set(gauge_value(counted.buckets));
        self.objects
            .get_or_create(&Unlabelled)
            .set(gauge_value(counted.objects));
        self.bytes
            .get_or_create(&Unlabelled)
            .set(gauge_value(counted.bytes));
        self.usage_taken
            .get_or_create(&Unlabelled)
            .set(counted.taken_secs);
    }

    /// Sets the open uploads counted.
    pub fn uploads(&self, open: u64) {
        self.uploads
            .get_or_create(&Unlabelled)
            .set(gauge_value(open));
    }

    /// Counts one run of `daemon` that ended as `outcome`.
    pub fn run(&self, daemon: &'static str, outcome: RunOutcome) {
        self.runs
            .get_or_create(&RunLabels {
                daemon,
                outcome: outcome.label(),
            })
            .inc();
    }
}
