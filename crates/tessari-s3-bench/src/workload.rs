//! Phases: for each size class and concurrency step, PUT, GET, HEAD, LIST and DELETE over the same keys, each
//! measured by `concurrency` tasks pulling operation numbers from one counter.

use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use reqwest::Method;
use tessari_s3_constants::{BENCH_INFLIGHT_BYTES, BENCH_LIST_OPS, BENCH_PHASE_BYTES};
use tokio::task::JoinSet;

use crate::body::SizeClass;
use crate::client::S3Client;
use crate::error::{Error, Result};
use crate::measure::{OpOutcome, Recorder};
use crate::ops::{self, Digests, Target};
use crate::report::Phase;

/// What to measure.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The bucket (created if absent).
    pub bucket: String,
    /// Size classes, in order.
    pub classes: Vec<SizeClass>,
    /// Concurrency steps, in order.
    pub concurrency: Vec<usize>,
    /// Operations per phase before the byte budget.
    pub ops: usize,
    /// Operations left out at the start of each phase.
    pub warmup: usize,
    /// Prefixes the keys are spread over.
    pub prefixes: usize,
}

/// The operations a phase runs for a class at a concurrency, and how many of them are warmup. The measured count
/// is the configured one capped by the byte budget, never fewer than two per task. Warmup is the configured count,
/// except for a class the byte budget limits: there one operation per task opens each connection.
#[must_use]
pub fn ops_for(plan: &Plan, class: SizeClass, concurrency: usize) -> (usize, usize) {
    let budget = BENCH_PHASE_BYTES
        .checked_div(class.bytes())
        .and_then(|ops| usize::try_from(ops).ok())
        .unwrap_or(usize::MAX);
    let measured = plan.ops.min(budget).max(concurrency.saturating_mul(2));
    let warmup = if budget < plan.ops {
        concurrency
    } else {
        plan.warmup
    };
    (measured.saturating_add(warmup), warmup)
}

/// Runs every phase of the plan and returns them in order.
///
/// # Errors
/// [`Error::Histogram`] or [`Error::Task`]; a failed request is counted, not returned.
pub async fn run(client: Arc<S3Client>, plan: &Plan) -> Result<Vec<Phase>> {
    let mut phases = Vec::new();
    for &class in &plan.classes {
        for &concurrency in &plan.concurrency {
            let inflight = class
                .bytes()
                .saturating_mul(u64::try_from(concurrency).unwrap_or(u64::MAX));
            if inflight > BENCH_INFLIGHT_BYTES {
                phases.push(Phase::skipped(
                    class,
                    concurrency,
                    "in-flight bytes above the harness's cap",
                ));
                continue;
            }
            let (total, warmup) = ops_for(plan, class, concurrency);
            let keys: Arc<[String]> = (0..total)
                .map(|i| {
                    let prefix = i.checked_rem(plan.prefixes.max(1)).unwrap_or(0);
                    format!("p{prefix:03}/{}/c{concurrency}/{i:08}", class.key_part())
                })
                .collect();
            let digests: Digests = (0..total).map(|_| OnceLock::new()).collect();
            let target = |index: usize| Target {
                client: Arc::clone(&client),
                bucket: plan.bucket.clone(),
                class,
                index,
                key: keys.get(index).cloned().unwrap_or_default(),
            };
            for op in ["PUT", "GET", "HEAD", "LIST", "DELETE"] {
                let (recorder, wall) = match op {
                    "PUT" => {
                        phase(concurrency, total, warmup, |i| {
                            ops::put(target(i), Arc::clone(&digests))
                        })
                        .await?
                    }
                    "GET" => {
                        phase(concurrency, total, warmup, |i| {
                            ops::get(target(i), Arc::clone(&digests))
                        })
                        .await?
                    }
                    "HEAD" => {
                        phase(concurrency, total, warmup, |i| {
                            ops::bodiless(target(i), Method::HEAD)
                        })
                        .await?
                    }
                    "LIST" => {
                        let lists = BENCH_LIST_OPS.saturating_add(warmup);
                        phase(concurrency, lists, warmup, |i| {
                            let prefix = i.checked_rem(plan.prefixes.max(1)).unwrap_or(0);
                            let prefix =
                                format!("p{prefix:03}/{}/c{concurrency}/", class.key_part());
                            ops::list(Arc::clone(&client), plan.bucket.clone(), prefix)
                        })
                        .await?
                    }
                    _ => {
                        phase(concurrency, total, warmup, |i| {
                            ops::bodiless(target(i), Method::DELETE)
                        })
                        .await?
                    }
                };
                let bytes_per_op = if matches!(op, "PUT" | "GET") {
                    class.bytes()
                } else {
                    0
                };
                let phase = Phase::measured(class, op, concurrency, &recorder, wall, bytes_per_op);
                tracing::info!(
                    class = class.key_part(),
                    op,
                    concurrency,
                    ops = phase.ops,
                    p50_us = phase.p50_us,
                    p99_us = phase.p99_us,
                    "phase measured"
                );
                phases.push(phase);
            }
        }
    }
    Ok(phases)
}

/// Runs operations `0..total` on `concurrency` tasks; operations below `warmup` are run and not recorded. The wall
/// time runs from the first recorded operation's start to the last task's end.
async fn phase<F, Fut>(
    concurrency: usize,
    total: usize,
    warmup: usize,
    op: F,
) -> Result<(Recorder, Duration)>
where
    F: Fn(usize) -> Fut,
    Fut: Future<Output = OpOutcome> + Send + 'static,
{
    let mut next = 0_usize;
    let mut measured_from: Option<Instant> = None;
    let (sender, mut receiver) =
        tokio::sync::mpsc::channel::<(usize, OpOutcome)>(concurrency.max(1));
    let mut tasks = JoinSet::new();
    // Operations are started here, one per free task slot, so `op` need not be Send or 'static; each runs as its
    // own task and reports back, and this loop is the only writer of the recorder.
    let mut running = 0_usize;
    let mut recorder = Recorder::new()?;
    loop {
        while running < concurrency.max(1) {
            let index = next;
            if index >= total {
                break;
            }
            next = next.saturating_add(1);
            if index == warmup {
                measured_from = Some(Instant::now());
            }
            let future = op(index);
            let sender = sender.clone();
            tasks.spawn(async move {
                let outcome = future.await;
                // The receiver lives until every task has reported.
                let _sent = sender.send((index, outcome)).await;
            });
            running = running.saturating_add(1);
        }
        if running == 0 {
            break;
        }
        let Some((index, outcome)) = receiver.recv().await else {
            return Err(Error::Task);
        };
        running = running.saturating_sub(1);
        if index >= warmup {
            recorder.record(&outcome);
        }
    }
    while let Some(joined) = tasks.join_next().await {
        joined.map_err(|_| Error::Task)?;
    }
    let wall = measured_from.map_or(Duration::ZERO, |started| started.elapsed());
    Ok((recorder, wall))
}

#[cfg(test)]
mod tests {
    use super::{Plan, ops_for};
    use crate::body::SizeClass;

    fn plan(ops: usize) -> Plan {
        Plan {
            bucket: "b".to_owned(),
            classes: Vec::new(),
            concurrency: Vec::new(),
            ops,
            warmup: 50,
            prefixes: 16,
        }
    }

    #[test]
    fn a_large_class_runs_fewer_operations_but_never_fewer_than_two_per_task() {
        // (total, warmup): the configured count plus the configured warmup.
        assert_eq!(ops_for(&plan(1000), SizeClass::Small, 4), (1050, 50));
        // 4 GiB budget / 64 MiB = 64 measured, one warmup per task.
        assert_eq!(ops_for(&plan(1000), SizeClass::Huge, 1), (65, 1));
        // 4 GiB / 256 MiB = 16 measured, raised to 2 per task at 16 tasks; 16 warmup.
        assert_eq!(ops_for(&plan(1000), SizeClass::Multipart, 16), (48, 16));
    }
}
