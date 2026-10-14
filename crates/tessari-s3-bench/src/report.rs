//! The report: one row per phase, headed by everything needed to compare it like-for-like.

use std::fmt::Write as _;
use std::time::Duration;

use crate::body::SizeClass;
use crate::measure::Recorder;

/// One measured (or skipped) phase.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Phase {
    /// The size class.
    pub class: SizeClass,
    /// `PUT`, `GET`, `HEAD`, `LIST` or `DELETE`.
    pub op: String,
    /// Tasks issuing requests at once.
    pub concurrency: usize,
    /// Successful measured operations.
    pub ops: u64,
    /// Seconds from the first measured operation to the end of the phase.
    pub wall_secs: f64,
    /// Successful measured operations per second.
    pub ops_per_sec: f64,
    /// Object bytes moved per second (PUT and GET).
    pub mib_per_sec: f64,
    /// Latency percentiles in microseconds.
    pub p50_us: u64,
    /// 95th percentile.
    pub p95_us: u64,
    /// 99th percentile.
    pub p99_us: u64,
    /// 99.9th percentile.
    pub p999_us: u64,
    /// Slowest.
    pub max_us: u64,
    /// Failed measured operations by label (`503 SlowDown`, `transport`, …).
    pub failures: std::collections::BTreeMap<String, u64>,
    /// GET bodies that did not match what was PUT.
    pub mismatches: u64,
    /// Share of the tasks' time spent preparing bodies (near 1.0 means the client, not the server, is the limit).
    pub client_prep_share: f64,
    /// Why the phase was not run, if it was not.
    pub skipped: Option<String>,
}

impl Phase {
    /// A phase that was not run.
    #[must_use]
    pub fn skipped(class: SizeClass, concurrency: usize, reason: &str) -> Self {
        Self {
            class,
            op: "*".to_owned(),
            concurrency,
            ops: 0,
            wall_secs: 0.0,
            ops_per_sec: 0.0,
            mib_per_sec: 0.0,
            p50_us: 0,
            p95_us: 0,
            p99_us: 0,
            p999_us: 0,
            max_us: 0,
            failures: std::collections::BTreeMap::new(),
            mismatches: 0,
            client_prep_share: 0.0,
            skipped: Some(reason.to_owned()),
        }
    }

    /// A phase from its recorder and wall time; `bytes_per_op` is the object size for PUT and GET, else 0.
    #[must_use]
    pub fn measured(
        class: SizeClass,
        op: &str,
        concurrency: usize,
        recorder: &Recorder,
        wall: Duration,
        bytes_per_op: u64,
    ) -> Self {
        let histogram = &recorder.histogram;
        let ops = histogram.len();
        let wall_secs = wall.as_secs_f64();
        let ops_f = to_f64(ops);
        let ops_per_sec = if wall_secs > 0.0 {
            ops_f / wall_secs
        } else {
            0.0
        };
        let mib_per_sec = ops_per_sec * to_f64(bytes_per_op) / (1024.0 * 1024.0);
        let task_time = wall_secs * to_f64(u64::try_from(concurrency).unwrap_or(u64::MAX));
        let client_prep_share = if task_time > 0.0 {
            recorder.prep.as_secs_f64() / task_time
        } else {
            0.0
        };
        Self {
            class,
            op: op.to_owned(),
            concurrency,
            ops,
            wall_secs,
            ops_per_sec,
            mib_per_sec,
            p50_us: histogram.value_at_quantile(0.50),
            p95_us: histogram.value_at_quantile(0.95),
            p99_us: histogram.value_at_quantile(0.99),
            p999_us: histogram.value_at_quantile(0.999),
            max_us: histogram.max(),
            failures: recorder.failures.clone(),
            mismatches: recorder.mismatches,
            client_prep_share,
            skipped: None,
        }
    }
}

/// A count as a float for rates, from its two 32-bit halves (exact below 2^53).
fn to_f64(count: u64) -> f64 {
    let high = u32::try_from(count >> 32).unwrap_or(u32::MAX);
    let low = u32::try_from(count & 0xFFFF_FFFF).unwrap_or(u32::MAX);
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}

/// Where the run happened.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Host {
    /// Operating system of the client.
    pub os: String,
    /// CPU architecture of the client.
    pub arch: String,
    /// Logical CPUs the client sees.
    pub cpus: usize,
    /// The operator's description of the hardware and the server's deployment (cluster shape, drives, VM).
    pub note: String,
}

/// One run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Report {
    /// The harness's version.
    pub bench_version: String,
    /// The server's build as the operator states it (image tag or digest, or git commit).
    pub server_build: String,
    /// `host:port` measured.
    pub endpoint: String,
    /// Region signed for.
    pub region: String,
    /// The client host.
    pub host: Host,
    /// Start of the run, seconds since the epoch.
    pub started_unix: u64,
    /// The bucket.
    pub bucket: String,
    /// Operations per phase before the byte budget.
    pub ops: usize,
    /// Configured warmup operations per phase (a class the byte budget limits warms one operation per task).
    pub warmup: usize,
    /// Prefixes the keys were spread over.
    pub prefixes: usize,
    /// Every phase, in run order.
    pub phases: Vec<Phase>,
}

impl Report {
    /// A run is valid only if every GET returned what was PUT.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.phases.iter().all(|phase| phase.mismatches == 0)
    }

    /// The report as a Markdown table.
    #[must_use]
    pub fn markdown(&self) -> String {
        let mut out = String::new();
        // Writing to a String cannot fail.
        let _header = writeln!(
            out,
            "server `{}` at `{}` · bench {} · client {} {} × {} CPUs · {} · ops {} warmup {} prefixes {} · valid: {}\n",
            self.server_build,
            self.endpoint,
            self.bench_version,
            self.host.os,
            self.host.arch,
            self.host.cpus,
            self.host.note,
            self.ops,
            self.warmup,
            self.prefixes,
            self.valid()
        );
        out.push_str("| class | op | conc | ops | ops/s | MiB/s | p50 ms | p95 ms | p99 ms | p99.9 ms | max ms | failures | mismatches | prep |\n");
        out.push_str("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|\n");
        for phase in &self.phases {
            if let Some(reason) = &phase.skipped {
                let _row = writeln!(
                    out,
                    "| {:?} | — | {} | skipped: {reason} |||||||||||",
                    phase.class, phase.concurrency
                );
                continue;
            }
            let failures: Vec<String> = phase
                .failures
                .iter()
                .map(|(label, count)| format!("{label}×{count}"))
                .collect();
            let _row = writeln!(
                out,
                "| {:?} | {} | {} | {} | {:.1} | {:.1} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {} | {} | {:.2} |",
                phase.class,
                phase.op,
                phase.concurrency,
                phase.ops,
                phase.ops_per_sec,
                phase.mib_per_sec,
                millis(phase.p50_us),
                millis(phase.p95_us),
                millis(phase.p99_us),
                millis(phase.p999_us),
                millis(phase.max_us),
                failures.join(" "),
                phase.mismatches,
                phase.client_prep_share
            );
        }
        out
    }
}

/// Microseconds as milliseconds for display.
#[must_use]
pub fn millis(micros: u64) -> f64 {
    to_f64(micros) / 1000.0
}
