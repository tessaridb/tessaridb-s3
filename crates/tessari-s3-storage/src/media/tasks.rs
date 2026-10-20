//! The rendition queues `media_images` and `media_videos` in TessariDB. A task is named after the version it renders
//! and its output, so the queues' `DEDUPLICATE` window makes queuing it twice a no-op; consumer names are checked
//! before they are quoted, because a name is grammar and is never bound.

use tessari_s3_infrastructure::tessaridb::{Answer, Number, RefusalClass, Value};
use uuid::Uuid;

use super::model::{Rendition, Task, TaskQueue};
use super::repository::TaskRepository;
use super::tessaridb::TessariMedia;
use crate::topic_groups::is_plain_name;
use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "media task",
        reason,
    }
}

/// `<source uuid>-<kind>-<size>-<format>`.
fn name(rendition: &Rendition) -> String {
    format!(
        "{}-{}",
        Uuid::from_bytes(rendition.source).hyphenated(),
        rendition.output.name().replace('/', "-")
    )
}

/// `'<consumer>'`, checked.
fn quoted(consumer: &str) -> Result<String> {
    if is_plain_name(consumer) {
        Ok(format!("'{consumer}'"))
    } else {
        Err(malformed(
            "a consumer name is 1 to 64 letters, digits, '-' or '_'",
        ))
    }
}

fn named(task: &Task) -> Vec<(String, Value)> {
    vec![("name".to_owned(), Value::String(task.name.clone()))]
}

impl TaskRepository for TessariMedia {
    async fn enqueue(&self, rendition: &Rendition) -> Result<()> {
        let table = TaskQueue::of(rendition.output).table();
        let version_id = rendition
            .version_id
            .clone()
            .map_or(Value::None, Value::String);
        let parameters = vec![
            ("name".to_owned(), Value::String(name(rendition))),
            ("bucket".to_owned(), Value::String(rendition.bucket.clone())),
            ("key".to_owned(), Value::String(rendition.key.clone())),
            ("version_id".to_owned(), version_id),
            ("source".to_owned(), Value::Uuid(rendition.source)),
            ("output".to_owned(), Value::String(rendition.output.name())),
        ];
        // A deduplicated create writes nothing and answers no record, which is success here.
        let script = format!(
            "CREATE {table}:$name = {{ bucket_name: $bucket, key: $key, version_id: $version_id, source: $source, \
             output: $output, retry_at: time::now() }};"
        );
        self.pool.run(&script, parameters).await?;
        Ok(())
    }

    async fn claim(&self, queue: TaskQueue, consumer: &str, limit: usize) -> Result<Vec<Task>> {
        let script = format!(
            "USE CONSUMER {}; CLAIM $limit FROM {};",
            quoted(consumer)?,
            queue.table()
        );
        let parameters = vec![("limit".to_owned(), crate::params::count(limit))];
        match self.pool.run(&script, parameters).await {
            Ok(answers) => tasks(queue, answers),
            // Two workers reaching for one head: the loser asks again next time, as for an empty queue.
            Err(error) if error.is_class(RefusalClass::Retry) => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    async fn remove(&self, task: &Task) -> Result<()> {
        let script = format!("DELETE {}:$name;", task.queue.table());
        self.pool.run(&script, named(task)).await?;
        Ok(())
    }

    async fn release(&self, task: &Task, error: &str, delay_secs: u64) -> Result<()> {
        let table = task.queue.table();
        // Named, not bare: a pooled connection is not the session that claimed, so only the group's form reaches it.
        let holder = match &task.consumer {
            Some(consumer) => format!(" FOR CONSUMER {}", quoted(consumer)?),
            None => String::new(),
        };
        let mut parameters = named(task);
        parameters.push(("error".to_owned(), Value::String(error.to_owned())));
        parameters.push(("delay".to_owned(), crate::params::seconds(delay_secs)));
        let script = format!(
            "BEGIN; UPDATE {table}:$name SET error = $error; \
             RELEASE {table}:$name{holder} NOT BEFORE duration::from_secs($delay); COMMIT;"
        );
        self.pool.run(&script, parameters).await?;
        Ok(())
    }

    async fn release_all(&self, queue: TaskQueue, consumer: &str) -> Result<()> {
        let script = format!(
            "RELEASE ALL FROM {} FOR CONSUMER {};",
            queue.table(),
            quoted(consumer)?
        );
        self.pool.run(&script, Vec::new()).await?;
        Ok(())
    }

    async fn spent(&self, queue: TaskQueue, attempts: i64, limit: usize) -> Result<Vec<Task>> {
        let script = format!(
            "SELECT * FROM {} WHERE attempts >= $attempts LIMIT $limit;",
            queue.table()
        );
        let parameters = vec![
            (
                "attempts".to_owned(),
                Value::Number(Number::Integer(attempts)),
            ),
            ("limit".to_owned(), crate::params::count(limit)),
        ];
        tasks(queue, self.pool.run(&script, parameters).await?)
    }
}

/// The tasks in the last answer of a script.
fn tasks(queue: TaskQueue, answers: Vec<Answer>) -> Result<Vec<Task>> {
    let Some(Answer::Records { records, .. }) = answers.into_iter().last() else {
        return Ok(Vec::new());
    };
    records
        .into_iter()
        .map(|(_, record)| task(queue, record))
        .collect()
}

fn task(queue: TaskQueue, record: Value) -> Result<Task> {
    let Value::Object(fields) = record else {
        return Err(malformed("not an object"));
    };
    let text = |field: &'static str| match fields.get(field) {
        Some(Value::String(text)) => Ok(Some(text.clone())),
        None | Some(Value::None | Value::Null) => Ok(None),
        Some(_) => Err(malformed(field)),
    };
    let required = |field: &'static str| text(field)?.ok_or_else(|| malformed(field));
    let Some(Value::Uuid(source)) = fields.get("source") else {
        return Err(malformed("source"));
    };
    let output = tessari_s3_core::media::Output::parse(&required("output")?)
        .map_err(|_| malformed("output"))?;
    let consumer = match fields.get("claimed_by") {
        Some(Value::Object(holder)) => match holder.get("consumer") {
            Some(Value::String(consumer)) => Some(consumer.clone()),
            _ => None,
        },
        _ => None,
    };
    let rendition = Rendition {
        bucket: required("bucket_name")?,
        key: required("key")?,
        version_id: text("version_id")?,
        source: *source,
        output,
    };
    Ok(Task {
        name: name(&rendition),
        queue,
        consumer,
        attempts: match fields.get("attempts") {
            Some(Value::Number(Number::Integer(attempts))) => *attempts,
            _ => 0,
        },
        error: text("error")?,
        rendition,
    })
}
