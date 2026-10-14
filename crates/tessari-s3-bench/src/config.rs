//! The harness's configuration, from the environment. An error names the variable and never echoes its value.

use tessari_s3_constants::{
    BENCH_DEFAULT_OPS, BENCH_DEFAULT_PREFIXES, BENCH_DEFAULT_WARMUP, DEFAULT_REGION,
};
use tessari_s3_types::SecretKey;

use crate::body::SizeClass;
use crate::error::{Error, Result};
use crate::workload::Plan;

/// Everything a run reads from its environment.
#[derive(Debug)]
pub struct BenchConfig {
    /// `TESSARIDB_S3_BENCH_ENDPOINT` — `host:port` of the S3 API (required).
    pub endpoint: String,
    /// `TESSARIDB_S3_BENCH_REGION` — the region to sign for.
    pub region: String,
    /// `TESSARIDB_S3_BENCH_ACCESS_KEY` — the access key (required).
    pub access_key: String,
    /// `TESSARIDB_S3_BENCH_SECRET_KEY` — its secret (required).
    pub secret_key: SecretKey,
    /// `TESSARIDB_S3_BENCH_SERVER_BUILD` — the server's build, written into the report (required).
    pub server_build: String,
    /// `TESSARIDB_S3_BENCH_HOST_NOTE` — the hardware and deployment, written into the report.
    pub host_note: String,
    /// `TESSARIDB_S3_BENCH_OUT` — the directory the JSON report is written to.
    pub out_dir: std::path::PathBuf,
    /// Bucket, classes, concurrency steps, operation counts, prefixes.
    pub plan: Plan,
}

impl BenchConfig {
    /// Reads the process environment.
    ///
    /// # Errors
    /// [`Error::MissingConfig`] or [`Error::InvalidConfig`] naming the variable.
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads through `lookup`, so tests supply variables without the process environment.
    ///
    /// # Errors
    /// As [`BenchConfig::from_env`].
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let required = |key: &'static str| get(key).ok_or(Error::MissingConfig { key });
        let bucket =
            get("TESSARIDB_S3_BENCH_BUCKET").unwrap_or_else(|| "tessaridb-s3-bench".to_owned());
        let bucket_ok = (3..=63).contains(&bucket.len())
            && bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !bucket_ok {
            return Err(Error::InvalidConfig {
                key: "TESSARIDB_S3_BENCH_BUCKET",
                reason: "3 to 63 lowercase letters, digits or hyphens",
            });
        }
        let classes = match get("TESSARIDB_S3_BENCH_CLASSES") {
            None => vec![
                SizeClass::Small,
                SizeClass::Medium,
                SizeClass::Large,
                SizeClass::Huge,
                SizeClass::Multipart,
            ],
            Some(list) => list
                .split(',')
                .map(SizeClass::parse)
                .collect::<Option<Vec<_>>>()
                .filter(|classes| !classes.is_empty())
                .ok_or(Error::InvalidConfig {
                    key: "TESSARIDB_S3_BENCH_CLASSES",
                    reason: "a comma-separated list of 4k, 128k, 1m, 64m, multipart",
                })?,
        };
        let concurrency = match get("TESSARIDB_S3_BENCH_CONCURRENCY") {
            None => vec![1, 4, 16, 64],
            Some(list) => list
                .split(',')
                .map(|step| {
                    step.trim()
                        .parse::<usize>()
                        .ok()
                        .filter(|n| (1..=1024).contains(n))
                })
                .collect::<Option<Vec<_>>>()
                .filter(|steps| !steps.is_empty())
                .ok_or(Error::InvalidConfig {
                    key: "TESSARIDB_S3_BENCH_CONCURRENCY",
                    reason: "a comma-separated list of whole numbers from 1 to 1024",
                })?,
        };
        let count = |key: &'static str, default: usize| -> Result<usize> {
            get(key).map_or(Ok(default), |text| {
                text.parse::<usize>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or(Error::InvalidConfig {
                        key,
                        reason: "not a positive whole number",
                    })
            })
        };
        let plan = Plan {
            bucket,
            classes,
            concurrency,
            ops: count("TESSARIDB_S3_BENCH_OPS", BENCH_DEFAULT_OPS)?,
            warmup: count("TESSARIDB_S3_BENCH_WARMUP", BENCH_DEFAULT_WARMUP)?,
            prefixes: count("TESSARIDB_S3_BENCH_PREFIXES", BENCH_DEFAULT_PREFIXES)?,
        };
        Ok(Self {
            endpoint: required("TESSARIDB_S3_BENCH_ENDPOINT")?,
            region: get("TESSARIDB_S3_BENCH_REGION").unwrap_or_else(|| DEFAULT_REGION.to_owned()),
            access_key: required("TESSARIDB_S3_BENCH_ACCESS_KEY")?,
            secret_key: SecretKey::new(required("TESSARIDB_S3_BENCH_SECRET_KEY")?),
            server_build: required("TESSARIDB_S3_BENCH_SERVER_BUILD")?,
            host_note: get("TESSARIDB_S3_BENCH_HOST_NOTE").unwrap_or_default(),
            out_dir: get("TESSARIDB_S3_BENCH_OUT")
                .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from),
            plan,
        })
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
