//! Deleting an object only while it is the one the caller saw.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Value};

use super::is_condition_refusal;
use crate::Result;

/// Removes the record `id` only while it is of `incarnation` and its ETag is `etag`; `false` when that did not hold or
/// there was no record.
pub(super) async fn remove_if(
    pool: &MetaPool,
    id: Value,
    incarnation: [u8; 16],
    etag: &str,
) -> Result<bool> {
    let parameters = vec![
        ("id".to_owned(), id),
        ("etag".to_owned(), Value::String(etag.to_owned())),
        ("incarnation".to_owned(), Value::Uuid(incarnation)),
    ];
    // A delete takes no condition, so the compare-and-set is an `UPDATE … WHERE` that changes nothing, in the delete's
    // transaction: it refuses the whole transaction when the record moved on. Unchanged data fires no `supersede`; the
    // delete itself queues the data, as any delete does.
    let script = "BEGIN; UPDATE objects:$id SET etag = $etag WHERE etag = $etag AND incarnation = $incarnation; \
                  DELETE objects:$id; COMMIT;";
    match pool.run(script, parameters).await {
        Ok(_) => Ok(true),
        Err(error) if is_condition_refusal(&error) => Ok(false),
        Err(error) => Err(error.into()),
    }
}
