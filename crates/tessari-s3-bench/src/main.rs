//! `tessaridb-s3-bench run` measures the server named by the environment and writes a report;
//! `tessaridb-s3-bench summarise <report.json>…` summarises like-for-like runs. Composition root only.

use tessari_s3_bench::{BenchConfig, read_reports, run, summarise};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tessari_s3_observability::init();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("run") => {
            let config = BenchConfig::from_env()?;
            let bucket = config.plan.bucket.clone();
            tokio::select! {
                biased;
                signal = shutdown_signal() => {
                    signal?;
                    // Objects already written stay under the bucket; their keys start with `p` and the class name.
                    tracing::warn!(bucket = %bucket, "interrupted; the run's objects are left in the bucket");
                    anyhow::bail!("interrupted");
                }
                result = run(config) => {
                    let (report, path) = result?;
                    println!("{}", report.markdown());
                    tracing::info!(path = %path.display(), valid = report.valid(), "report written");
                    anyhow::ensure!(report.valid(), "a GET returned bytes that differ from the PUT");
                }
            }
        }
        Some("summarise") => {
            let paths: Vec<std::path::PathBuf> = args.map(std::path::PathBuf::from).collect();
            println!("{}", summarise(&read_reports(&paths)?)?);
        }
        _ => anyhow::bail!(
            "usage: tessaridb-s3-bench run | tessaridb-s3-bench summarise <report.json>…"
        ),
    }
    Ok(())
}

/// Resolves on SIGINT or SIGTERM.
async fn shutdown_signal() -> anyhow::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        biased;
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    Ok(())
}
