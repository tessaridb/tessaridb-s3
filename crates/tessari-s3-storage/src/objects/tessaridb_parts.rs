//! The parts of a multipart object, read with the object in one snapshot.

use tessari_s3_constants::MULTIPART_MAX_PARTS;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};

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
    })
}

/// The parts of upload `upload` in number order, if the object record `object` is still the one completed from it.
pub(super) async fn parts_of(
    pool: &MetaPool,
    object: Value,
    upload: [u8; 16],
) -> Result<Option<Vec<Part>>> {
    let parameters = vec![
        ("id".to_owned(), object),
        ("upload".to_owned(), Value::Uuid(upload)),
    ];
    // One snapshot: the object still on this upload, and the upload's parts. `LIMIT` takes a literal; this is the
    // server's own constant.
    let script = format!(
        "BEGIN; SELECT upload FROM ONLY objects:$id; \
         SELECT number, data, size FROM parts WHERE upload = $upload LIMIT {MULTIPART_MAX_PARTS} \
         USING INDEX by_upload; COMMIT;"
    );
    let mut answers = pool.run(&script, parameters).await?.into_iter();
    let _begin = answers.next();
    let still = matches!(
        first_record(answers.next())?,
        Some(Value::Object(fields)) if fields.get("upload") == Some(&Value::Uuid(upload))
    );
    if !still {
        return Ok(None);
    }
    let records = match answers.next() {
        Some(Answer::Records { records, .. }) => records,
        _ => Vec::new(),
    };
    let mut parts = records
        .iter()
        .map(|(_, value)| part(value))
        .collect::<Result<Vec<Part>>>()?;
    parts.sort_unstable_by_key(|part| part.number);
    Ok(Some(parts))
}
