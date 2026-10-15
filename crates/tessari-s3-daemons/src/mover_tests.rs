use std::future::pending;
use std::time::Duration;

use tokio::sync::watch;

use super::until_stopped;

#[tokio::test(start_paused = true)]
async fn work_that_finishes_answers_with_its_result() {
    let (_stop, stopped) = watch::channel(false);
    assert_eq!(until_stopped(stopped, async { 7 }).await, Some(7));
}

#[tokio::test(start_paused = true)]
async fn shutdown_during_the_work_stops_it() {
    let (stop, stopped) = watch::channel(false);
    let work = tokio::spawn(until_stopped(stopped, pending::<u8>()));
    tokio::time::sleep(Duration::from_secs(3600)).await;
    stop.send(true).expect("listening");
    assert_eq!(work.await.expect("returns"), None);
}

#[tokio::test(start_paused = true)]
async fn work_does_not_start_after_shutdown_even_when_it_is_ready() {
    // Ready work beside a shutdown already given: only a shutdown checked first wins every time.
    for _ in 0..64 {
        let (stop, stopped) = watch::channel(false);
        stop.send(true).expect("listening");
        assert_eq!(until_stopped(stopped, async { 7 }).await, None);
    }
}

#[tokio::test(start_paused = true)]
async fn a_dropped_sender_stops_the_work_too() {
    let (stop, stopped) = watch::channel(false);
    drop(stop);
    let stopped = tokio::time::timeout(
        Duration::from_secs(3600),
        until_stopped(stopped, pending::<u8>()),
    )
    .await;
    assert_eq!(stopped, Ok(None), "the work is not waited on forever");
}
