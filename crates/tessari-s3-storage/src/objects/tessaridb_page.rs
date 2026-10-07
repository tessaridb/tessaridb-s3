//! The listing read: object records in record-id order from an anchor, with the bucket's incarnation, at one
//! snapshot.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};
use tessari_s3_types::BucketName;

use super::super::entity::list_row;
use super::super::repository::Batch;
use crate::Result;
use crate::answers::{first_record, incarnation_of};

/// What a listing reads of each record: no inline bytes.
const LIST_FIELDS: &str = "bucket_name, key, incarnation, size, etag, modified, checksums";

/// See `ObjectRepository::page`.
pub(super) async fn page(
    pool: &MetaPool,
    bucket: &BucketName,
    at: Option<&str>,
    after: &str,
    limit: usize,
) -> Result<Batch> {
    let in_bucket = |key: &str| Value::String(format!("{}/{key}", bucket.as_str()));
    let mut parameters = vec![
        (
            "bucket".to_owned(),
            Value::String(bucket.as_str().to_owned()),
        ),
        ("after".to_owned(), in_bucket(after)),
    ];
    // `AFTER` with no `ORDER BY` walks record ids from the anchor, which need not exist (a key deleted since the
    // page that named it), and is exclusive — so a start AT a key reads that record by id first. `LIMIT` takes a
    // literal on 0.33.1; the limit is this server's own integer, never a caller's value.
    let exact = match at {
        Some(key) => {
            parameters.push(("at".to_owned(), in_bucket(key)));
            format!("SELECT {LIST_FIELDS} FROM ONLY objects:$at;")
        }
        None => String::new(),
    };
    let script = format!(
        "BEGIN; SELECT incarnation FROM ONLY buckets:$bucket; {exact} \
         SELECT {LIST_FIELDS} FROM objects AFTER objects:$after LIMIT {limit}; COMMIT;"
    );
    let mut answers = pool.run(&script, parameters).await?.into_iter();
    let _begin = answers.next();
    let bucket_incarnation = incarnation_of(first_record(answers.next())?)?;
    let mut rows = Vec::new();
    if at.is_some()
        && let Some(value) = first_record(answers.next())?
    {
        rows.push(list_row(&value)?);
    }
    let walked = match answers.next() {
        Some(Answer::Records { records, .. }) => records,
        _ => Vec::new(),
    };
    let full = walked.len() >= limit;
    for (_, value) in &walked {
        rows.push(list_row(value)?);
    }
    Ok(Batch {
        bucket: bucket_incarnation,
        rows,
        full,
    })
}
