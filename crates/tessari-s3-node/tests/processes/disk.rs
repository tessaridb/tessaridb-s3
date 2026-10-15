//! What the fault tests read off a node's drive and do to its process: the files under its data directory, the
//! data shards an object put there, and a kill that stops it as a power cut would.

use std::path::Path;
use std::time::{Duration, Instant};

/// Files of any kind under `dir`.
fn files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() { files(&path) } else { 1 }
        })
        .sum()
}

/// The data shards (`.s0` to `.s3` of RS(4, 2)) under `dir`: the four a whole read asks for first.
pub fn first_shards(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .flat_map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                first_shards(&path)
            } else if [".s0", ".s1", ".s2", ".s3"]
                .iter()
                .any(|suffix| path.to_string_lossy().ends_with(suffix))
            {
                vec![path]
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// Kills process `pid` with SIGKILL, as a power cut would; the test still owns its handle and reaps it later.
pub async fn kill(pid: u32) {
    let status = tokio::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .await
        .expect("kill runs");
    assert!(status.success(), "process {pid} was not running");
}

/// Waits until a node has started writing its part of an upload: the first file appears in its data directory.
pub async fn writing(dir: &Path) {
    let started = Instant::now();
    while files(dir) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "{} never received a shard",
            dir.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
