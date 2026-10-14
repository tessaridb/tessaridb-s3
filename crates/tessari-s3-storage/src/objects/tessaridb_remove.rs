//! Deleting an object — with the version records the bucket's versioning state asks for — and, for the console,
//! only while it is the one the caller saw.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Value};
use tessari_s3_types::{BucketName, ObjectKey, Timestamp};

use super::super::repository::VersionPlan;
use super::{id, is_lost_or_refused, versions};
use crate::Result;

/// Removes the record at the key with `plan`'s version changes, as ONE transaction; with `held`, only while the record
/// is of `incarnation` and carries that ETag — and, when given, that write time. `false` when that did not hold, or
/// the key's versions moved on since the plan was read.
pub(super) async fn remove(
    pool: &MetaPool,
    bucket: &BucketName,
    key: &ObjectKey,
    incarnation: [u8; 16],
    held: Option<(&str, Option<Timestamp>)>,
    plan: &VersionPlan,
) -> Result<bool> {
    let mut parameters = vec![("id".to_owned(), id(bucket, key))];
    // A delete takes no condition, so the compare-and-set is an `UPDATE … WHERE` that changes nothing, in the delete's
    // transaction: it refuses the whole transaction when the record moved on. Unchanged data fires no `supersede`; the
    // delete itself queues the data, as any delete does — and a version still holding the file keeps it.
    let guard = match held {
        Some((etag, modified)) => {
            parameters.push(("etag".to_owned(), Value::String(etag.to_owned())));
            parameters.push(("incarnation".to_owned(), Value::Uuid(incarnation)));
            match modified {
                // A rewrite with the same bytes keeps the ETag and not the time.
                Some(at) => {
                    parameters.push((
                        "modified".to_owned(),
                        Value::Datetime {
                            seconds: at.seconds,
                            nanos: at.nanos,
                        },
                    ));
                    "UPDATE objects:$id SET etag = $etag \
                     WHERE etag = $etag AND incarnation = $incarnation AND modified = $modified; "
                }
                None => {
                    "UPDATE objects:$id SET etag = $etag WHERE etag = $etag AND incarnation = $incarnation; "
                }
            }
        }
        None => "",
    };
    let changes = versions::statements(bucket, key, incarnation, plan, false)?;
    parameters.extend(changes.parameters);
    let script = format!(
        "BEGIN; LET $now = time::now(); {guard}DELETE objects:$id; {}COMMIT;",
        changes.script
    );
    match pool.run(&script, parameters).await {
        Ok(_) => Ok(true),
        Err(error) if is_lost_or_refused(&error) => Ok(false),
        Err(error) => Err(error.into()),
    }
}
