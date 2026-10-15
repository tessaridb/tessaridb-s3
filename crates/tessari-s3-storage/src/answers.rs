//! Reading TessariDB answers: the first record of a read, every record of a listing, and a bucket's incarnation.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::{Error, Result};

/// The first record of a records answer.
pub(crate) fn first_record(answer: Option<Answer>) -> Result<Option<Value>> {
    match answer {
        Some(Answer::Records { records, .. }) => {
            Ok(records.into_iter().next().map(|(_, value)| value))
        }
        Some(Answer::Value {
            value: Value::None, ..
        })
        | None => Ok(None),
        Some(Answer::Value { value, .. }) => Ok(Some(value)),
        Some(_) => Err(Error::Malformed {
            record: "object",
            reason: "unexpected answer kind",
        }),
    }
}

/// The statement reading `fields` of bucket `$bucket`. Buckets live in the global database and the object metadata in
/// the region's, so a regional script names the bucket by its qualified name — which reads it inside the regional
/// transaction, as of the same moment.
pub(crate) fn bucket_read(pool: &MetaPool, fields: &str) -> String {
    format!(
        "SELECT {fields} FROM ONLY {}.buckets:$bucket;",
        pool.global_database()
    )
}

/// The bucket's incarnation from a [`bucket_read`] of `incarnation, region`, or `None` when there is no such bucket.
/// A bucket of another region than `own` is [`Error::OtherRegion`]: its object metadata is in that region's
/// database, so answering from this one would read nothing and write where its region never looks.
pub(crate) fn incarnation_of(value: Option<Value>, own: Option<&str>) -> Result<Option<[u8; 16]>> {
    match value {
        None => Ok(None),
        Some(Value::Object(fields)) => match fields.get("incarnation") {
            Some(Value::Uuid(bytes)) => {
                if let (Some(own), Some(Value::String(region))) = (own, fields.get("region"))
                    && region != own
                {
                    return Err(Error::OtherRegion {
                        region: region.clone(),
                    });
                }
                Ok(Some(*bytes))
            }
            _ => Err(Error::Malformed {
                record: "bucket",
                reason: "incarnation",
            }),
        },
        Some(_) => Err(Error::Malformed {
            record: "bucket",
            reason: "not an object",
        }),
    }
}

/// Every record of a records answer, as `record` names them in an error.
pub(crate) fn all_records(answer: Option<Answer>, record: &'static str) -> Result<Vec<Value>> {
    match answer {
        Some(Answer::Records { records, .. }) => {
            Ok(records.into_iter().map(|(_, value)| value).collect())
        }
        _ => Err(Error::Malformed {
            record,
            reason: "a listing answered no records",
        }),
    }
}
