//! Sweeps in TessariDB: `media_sweeps` holds a sweep under way, one record per bucket, and a pass holds it while it
//! plans one page; `media_swept` keeps the last finished one. Every advance is conditioned on the claimed `started`,
//! so a sweep started again while a page was worked wins.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaError, Number, RefusalClass, Value};

use super::model::{Sweep, SweepProgress};
use super::repository::SweepRepository;
use super::tessaridb::TessariMedia;
use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "media sweep",
        reason,
    }
}

/// A release of a record no longer there, or an advance whose `started` no longer holds.
fn moved(error: &MetaError) -> bool {
    error.is_class(RefusalClass::Conflict) || error.is_class(RefusalClass::Invalid)
}

fn integer(n: u64) -> Value {
    Value::Number(Number::Integer(i64::try_from(n).unwrap_or(i64::MAX)))
}

fn count(fields: &std::collections::BTreeMap<String, Value>, field: &'static str) -> Result<u64> {
    match fields.get(field) {
        Some(Value::Number(Number::Integer(n))) => u64::try_from(*n).map_err(|_| malformed(field)),
        _ => Err(malformed(field)),
    }
}

/// The first record of `answer`, if it holds one.
fn first(answer: Option<Answer>) -> Option<Value> {
    match answer {
        Some(Answer::Records { records, .. }) => records.into_iter().next().map(|(_, value)| value),
        _ => None,
    }
}

fn sweep(record: Value) -> Result<Sweep> {
    let Value::Object(fields) = record else {
        return Err(malformed("not an object"));
    };
    let text = |field: &'static str| match fields.get(field) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(field)),
    };
    let started = match fields.get("started") {
        Some(Value::Datetime { seconds, nanos }) => (*seconds, *nanos),
        _ => return Err(malformed("started")),
    };
    Ok(Sweep {
        bucket: text("name")?,
        after: text("after")?,
        listed: count(&fields, "listed")?,
        queued: count(&fields, "queued")?,
        started,
    })
}

impl SweepRepository for TessariMedia {
    async fn start(&self, bucket: &str) -> Result<()> {
        let script = "UPSERT media_sweeps:$bucket SET name = $bucket, after = '', listed = 0, queued = 0, \
                      started = time::now();";
        self.pool
            .run(
                script,
                vec![("bucket".to_owned(), Value::String(bucket.to_owned()))],
            )
            .await?;
        Ok(())
    }

    async fn claim(&self) -> Result<Option<Sweep>> {
        let answers = self
            .pool
            .run("CLAIM FROM media_sweeps;", Vec::new())
            .await?;
        first(answers.into_iter().next()).map(sweep).transpose()
    }

    async fn advance(&self, sweep: &Sweep, last: bool) -> Result<bool> {
        let parameters = vec![
            ("bucket".to_owned(), Value::String(sweep.bucket.clone())),
            ("after".to_owned(), Value::String(sweep.after.clone())),
            ("listed".to_owned(), integer(sweep.listed)),
            ("queued".to_owned(), integer(sweep.queued)),
            (
                "started".to_owned(),
                Value::Datetime {
                    seconds: sweep.started.0,
                    nanos: sweep.started.1,
                },
            ),
        ];
        let script = if last {
            "BEGIN; UPDATE media_sweeps:$bucket SET after = $after WHERE started = $started; \
             DELETE media_sweeps:$bucket; UPSERT media_swept:$bucket SET name = $bucket, listed = $listed, \
             queued = $queued, started = $started, finished = time::now(); COMMIT;"
        } else {
            "BEGIN; UPDATE media_sweeps:$bucket SET after = $after, listed = $listed, queued = $queued \
             WHERE started = $started; RELEASE media_sweeps:$bucket; COMMIT;"
        };
        match self.pool.run(script, parameters.clone()).await {
            Ok(_) => Ok(true),
            // Started again while this page was worked: hand the new one back at once rather than at the hold's end.
            Err(error) if moved(&error) => {
                match self
                    .pool
                    .run("RELEASE media_sweeps:$bucket;", parameters)
                    .await
                {
                    Ok(_) => Ok(false),
                    Err(error) if moved(&error) => Ok(false),
                    Err(error) => Err(error.into()),
                }
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn progress(&self, bucket: &str) -> Result<SweepProgress> {
        let answers = self
            .pool
            .run(
                "SELECT * FROM media_sweeps:$bucket; SELECT * FROM media_swept:$bucket;",
                vec![("bucket".to_owned(), Value::String(bucket.to_owned()))],
            )
            .await?;
        let mut answers = answers.into_iter();
        if let Some(running) = first(answers.next()) {
            let running = sweep(running)?;
            return Ok(SweepProgress::Running {
                listed: running.listed,
                queued: running.queued,
            });
        }
        let Some(Value::Object(fields)) = first(answers.next()) else {
            return Ok(SweepProgress::None);
        };
        Ok(SweepProgress::Done {
            listed: count(&fields, "listed")?,
            queued: count(&fields, "queued")?,
        })
    }
}
