use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::watch;

use super::{Runs, run};
use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// A daemon whose every run does what `behaviour` says, counting the runs it started.
struct Fake {
    behaviour: &'static str,
    started: Arc<AtomicU64>,
}

impl Daemon for Fake {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(10)
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
    fn run_once(&self) -> DaemonFuture<'_> {
        self.started.fetch_add(1, Ordering::Relaxed);
        let behaviour = self.behaviour;
        Box::pin(async move {
            match behaviour {
                "fail" => Err(Error::Run {
                    daemon: "fake",
                    message: "on purpose".to_owned(),
                }),
                "hang" => {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    Ok(Report {
                        message: "never".to_owned(),
                    })
                }
                _ => Ok(Report {
                    message: "done".to_owned(),
                }),
            }
        })
    }
}

/// Runs a fake for `ticks` intervals of virtual time, then signals shutdown and returns what the runner counted.
async fn drive(behaviour: &'static str, ticks: u64) -> (Runs, u64) {
    let started = Arc::new(AtomicU64::new(0));
    let daemon = Arc::new(Fake {
        behaviour,
        started: Arc::clone(&started),
    });
    let (stop, stopped) = watch::channel(false);
    let runner = tokio::spawn(run(daemon, stopped));
    // The first tick fires at once; each later one after the interval (plus a run's timeout when it hangs).
    tokio::time::sleep(Duration::from_secs(
        ticks.saturating_mul(15).saturating_add(1),
    ))
    .await;
    stop.send(true).expect("runner listening");
    let runs = runner.await.expect("runner returns");
    (runs, started.load(Ordering::Relaxed))
}

#[tokio::test(start_paused = true)]
async fn each_tick_runs_once_and_shutdown_stops_the_runner() {
    let (runs, started) = drive("ok", 3).await;
    assert!(runs.succeeded >= 3, "{runs:?}");
    assert_eq!((runs.failed, runs.timed_out), (0, 0));
    assert_eq!(runs.succeeded, started);
}

#[tokio::test(start_paused = true)]
async fn a_failing_run_is_counted_and_the_next_tick_runs_again() {
    let (runs, started) = drive("fail", 3).await;
    assert!(runs.failed >= 3, "{runs:?}");
    assert_eq!((runs.succeeded, runs.timed_out), (0, 0));
    assert_eq!(runs.failed, started);
}

#[tokio::test(start_paused = true)]
async fn a_run_that_hangs_is_abandoned_at_its_timeout() {
    let (runs, started) = drive("hang", 3).await;
    assert!(runs.timed_out >= 2, "{runs:?}");
    assert_eq!((runs.succeeded, runs.failed), (0, 0));
    assert_eq!(runs.timed_out, started);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_sender_also_stops_the_runner() {
    let daemon = Arc::new(Fake {
        behaviour: "ok",
        started: Arc::new(AtomicU64::new(0)),
    });
    let (stop, stopped) = watch::channel(false);
    let runner = tokio::spawn(run(daemon, stopped));
    tokio::time::sleep(Duration::from_secs(1)).await;
    drop(stop);
    let runs = tokio::time::timeout(Duration::from_secs(60), runner)
        .await
        .expect("the runner returned")
        .expect("no panic");
    assert_eq!(runs.succeeded, 1, "the first tick ran");
}
