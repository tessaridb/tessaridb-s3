//! The tracing subscriber every process installs first.

/// Installs the tracing subscriber: JSON lines, filtered by `RUST_LOG` (default `info`).
pub fn init() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .init();
}
