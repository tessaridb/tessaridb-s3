//! The parts of a multipart object, read with the object in one snapshot.

use tessari_s3_constants::MULTIPART_MAX_PARTS;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};

use super::super::entity::sealed;
use super::super::model::Part;
use crate::answers::first_record;
use crate::{Error, Result};

/// One parts record as an object reads it.
fn part(value: &Value) -> Result<Part> {
    let malformed = |reason| Error::Malformed {
        record: "part",
        reason,
    };
    let Value::Object(fields) = value else {
        return Err(malformed("not an object"));
    };
    let integer = |field: &'static str| match fields.get(field) {
        Some(Value::Number(Number::Integer(n))) => Ok(*n),
        _ => Err(malformed(field)),
    };
    Ok(Part {
        number: u16::try_from(integer("number")?).map_err(|_| malformed("number"))?,
        data: match fields.get("data") {
            Some(Value::Uuid(id)) => *id,
            _ => return Err(malformed("data")),
        },
        size: u64::try_from(integer("size")?).map_err(|_| malformed("size"))?,
        encryption: sealed(fields)?,
    })
}

/// The parts of upload `upload` in number order, if the record `record` of `table` (`objects` or `versions`) still
/// names it after they are read.
pub(super) async fn parts_of(
    pool: &MetaPool,
    table: &'static str,
    record: Value,
    upload: [u8; 16],
) -> Result<Option<Vec<Part>>> {
    let parameters = vec![
        ("id".to_owned(), record),
        ("upload".to_owned(), Value::Uuid(upload)),
        (
            "limit".to_owned(),
            crate::params::count(usize::from(MULTIPART_MAX_PARTS)),
        ),
    ];
    // Two plain reads, the parts FIRST: a completed upload's parts never change and go only after no object names
    // the upload, and an upload id never comes back — so the object still naming it afterwards proves the parts were
    // whole when they were read. A table is grammar, and this one is the server's own.
    let script = format!(
        "SELECT number, data, size, sse_alg, sse_kek, sse_nonce, sse_key FROM parts WHERE upload = $upload LIMIT $limit \
         USING INDEX by_upload; SELECT upload FROM ONLY {table}:$id;"
    );
    let mut answers = pool.run(&script, parameters).await?.into_iter();
    let records = match answers.next() {
        Some(Answer::Records { records, .. }) => records,
        _ => Vec::new(),
    };
    let still = matches!(
        first_record(answers.next())?,
        Some(Value::Object(fields)) if fields.get("upload") == Some(&Value::Uuid(upload))
    );
    if !still {
        return Ok(None);
    }
    let mut parts = records
        .iter()
        .map(|(_, value)| part(value))
        .collect::<Result<Vec<Part>>>()?;
    parts.sort_unstable_by_key(|part| part.number);
    Ok(Some(parts))
}
