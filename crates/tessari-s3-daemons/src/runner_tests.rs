use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tessari_s3_observability::Metrics;
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
                        idle: false,
                    })
                }
                "idle" => Ok(Report {
                    message: "nothing to do".to_owned(),
                    idle: true,
                }),
                _ => Ok(Report {
                    message: "done".to_owned(),
                    idle: false,
                }),
            }
        })
    }
}

/// Runs a fake for `ticks` intervals of virtual time, then signals shutdown and returns what the runner counted.
async fn drive(behaviour: &'static str, ticks: u64) -> (Runs, u64) {
    let (runs, started, _) = drive_counted(behaviour, ticks).await;
    (runs, started)
}

/// As [`drive`], with the metrics the runner counted into, encoded.
async fn drive_counted(behaviour: &'static str, ticks: u64) -> (Runs, u64, String) {
    let started = Arc::new(AtomicU64::new(0));
    let daemon = Arc::new(Fake {
        behaviour,
        started: Arc::clone(&started),
    });
    let (stop, stopped) = watch::channel(false);
    let metrics = Arc::new(Metrics::new());
    let runner = tokio::spawn(run(daemon, stopped, Arc::clone(&metrics)));
    // The first tick fires at once; each later one after the interval (plus a run's timeout when it hangs).
    tokio::time::sleep(Duration::from_secs(
        ticks.saturating_mul(15).saturating_add(1),
    ))
    .await;
    stop.send(true).expect("runner listening");
    let runs = runner.await.expect("runner returns");
    let text = metrics.encode().expect("encoded");
    (runs, started.load(Ordering::Relaxed), text)
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
    let runner = tokio::spawn(run(daemon, stopped, Arc::new(Metrics::new())));
    tokio::time::sleep(Duration::from_secs(1)).await;
    drop(stop);
    let runs = tokio::time::timeout(Duration::from_secs(60), runner)
        .await
        .expect("the runner returned")
        .expect("no panic");
    assert_eq!(runs.succeeded, 1, "the first tick ran");
}

/// What the runner logs at INFO and above while `behaviour` runs for three ticks.
async fn logged(behaviour: &'static str) -> String {
    let lines = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let sink = Arc::clone(&lines);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(move || Writer(Arc::clone(&sink)))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    drive(behaviour, 3).await;
    let bytes = lines.lock().expect("the log").clone();
    String::from_utf8(bytes).expect("text")
}

struct Writer(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("the log").extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_run_that_found_nothing_to_do_is_not_logged_at_info() {
    let idle = logged("idle").await;
    assert!(!idle.contains("daemon run completed"), "{idle}");
    let worked = logged("ok").await;
    assert!(
        worked.matches("daemon run completed").count() >= 3,
        "control: a run that did something is logged at INFO: {worked}"
    );
}

#[tokio::test(start_paused = true)]
async fn every_run_is_counted_in_the_metrics_by_its_outcome() {
    for (behaviour, outcome) in [
        ("ok", "succeeded"),
        ("fail", "failed"),
        ("hang", "timed_out"),
    ] {
        let (runs, _, text) = drive_counted(behaviour, 3).await;
        let counted = runs
            .succeeded
            .saturating_add(runs.failed)
            .saturating_add(runs.timed_out);
        assert!(counted > 0, "{runs:?}");
        let line =
            format!("s3_daemon_runs_total{{daemon=\"fake\",outcome=\"{outcome}\"}} {counted}");
        assert!(text.lines().any(|seen| seen == line), "{line} in\n{text}");
    }
}

/// A daemon whose interval is read from `secs` each time it is asked, counting its runs.
struct Paced {
    secs: Arc<AtomicU64>,
    started: Arc<AtomicU64>,
}

impl Daemon for Paced {
    fn name(&self) -> &'static str {
        "paced"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.secs.load(Ordering::Relaxed))
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
    fn run_once(&self) -> DaemonFuture<'_> {
        self.started.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            Ok(Report {
                message: "done".to_owned(),
                idle: false,
            })
        })
    }
}

#[tokio::test(start_paused = true)]
async fn an_interval_changed_while_running_paces_the_next_wait() {
    let secs = Arc::new(AtomicU64::new(100));
    let started = Arc::new(AtomicU64::new(0));
    let daemon = Arc::new(Paced {
        secs: Arc::clone(&secs),
        started: Arc::clone(&started),
    });
    let (stop, stopped) = watch::channel(false);
    let runner = tokio::spawn(run(daemon, stopped, Arc::new(Metrics::new())));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        started.load(Ordering::Relaxed),
        1,
        "the first run starts at once"
    );
    secs.store(1, Ordering::Relaxed);
    // The wait already begun under 100 s ends first; every wait after it is the new second.
    tokio::time::sleep(Duration::from_secs(110)).await;
    stop.send(true).expect("runner listening");
    runner.await.expect("runner returns");
    assert!(
        started.load(Ordering::Relaxed) >= 5,
        "runs after the change: {}",
        started.load(Ordering::Relaxed)
    );
}
