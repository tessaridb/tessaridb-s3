//! One run end to end: the bucket, every phase, the report on disk.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::S3Client;
use crate::config::BenchConfig;
use crate::error::Result;
use crate::report::{Host, Report};
use crate::workload;

/// Runs the configured plan and writes `bench-<start>.json` into the output directory.
///
/// # Errors
/// Set-up refusals, client construction, histogram creation, a failed worker or the report file.
pub async fn run(config: BenchConfig) -> Result<(Report, std::path::PathBuf)> {
    let client = Arc::new(S3Client::new(
        &config.endpoint,
        &config.region,
        &config.access_key,
        config.secret_key,
    )?);
    client.ensure_bucket(&config.plan.bucket).await?;
    let started_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let phases = workload::run(Arc::clone(&client), &config.plan).await?;
    let report = Report {
        bench_version: env!("CARGO_PKG_VERSION").to_owned(),
        server_build: config.server_build,
        endpoint: config.endpoint,
        region: config.region,
        host: Host {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            cpus: std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get),
            note: config.host_note,
        },
        started_unix,
        bucket: config.plan.bucket.clone(),
        ops: config.plan.ops,
        warmup: config.plan.warmup,
        prefixes: config.plan.prefixes,
        phases,
    };
    std::fs::create_dir_all(&config.out_dir)?;
    let path = config.out_dir.join(format!("bench-{started_unix}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    Ok((report, path))
}

/// Reads reports from `paths`.
///
/// # Errors
/// [`crate::Error::Io`] or [`crate::Error::Json`].
pub fn read_reports(paths: &[std::path::PathBuf]) -> Result<Vec<Report>> {
    paths
        .iter()
        .map(|path| Ok(serde_json::from_slice(&std::fs::read(path)?)?))
        .collect()
}
