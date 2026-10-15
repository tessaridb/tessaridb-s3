//! Writing an object: its `objects` record under the write's condition, the version records its bucket's versioning
//! state asks for, and the data or upload it points at — one transaction.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Value};
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::entity::record;
use super::super::model::{Content, NewObject};
use super::super::repository::{Guard, VersionPlan, Wrote};
use super::{id, is_lost_or_refused, versions};
use crate::answers::first_record;
use crate::{Error, Result};

impl super::TessariObjects {
    /// Whether upload `upload` still exists — no abort or other completion ended it.
    pub(crate) async fn upload_open(&self, upload: [u8; 16]) -> Result<bool> {
        let parameters = vec![("upload".to_owned(), Value::Uuid(upload))];
        let answers = self
            .pool
            .run("SELECT last_part FROM ONLY uploads:$upload;", parameters)
            .await?;
        Ok(first_record(answers.into_iter().next())?.is_some())
    }
}

/// See `ObjectRepository::write`.
pub(super) async fn write(
    pool: &MetaPool,
    bucket: &BucketName,
    key: &ObjectKey,
    incarnation: [u8; 16],
    object: &NewObject,
    guard: &Guard,
    plan: &VersionPlan,
) -> Result<Wrote> {
    let mut parameters = vec![
        ("id".to_owned(), id(bucket, key)),
        (
            "object".to_owned(),
            record(bucket.as_str(), key.as_str(), incarnation, object)?,
        ),
        ("incarnation".to_owned(), Value::Uuid(incarnation)),
    ];
    // `modified` is the store's clock at commit (`$now`, shared with the version record), merged into the bound
    // record with the version id the object answers with, once its bucket has been versioned.
    // `notify` and `op` are what the store's `object_created` event decides on: whether the bucket notifies, and which
    // operation the change is.
    let op = match (&object.content, object.copied) {
        (Content::Parts(_), _) => "CompleteMultipartUpload",
        (Content::Data(_), true) => "Copy",
        (Content::Data(_), false) => "Put",
    };
    parameters.push(("notify".to_owned(), Value::Bool(plan.notify)));
    parameters.push(("op".to_owned(), Value::String(op.to_owned())));
    // A version being replicated answers its status on the current object too.
    let replication = if plan.replicate.is_some() {
        ", replication_status: 'PENDING'"
    } else {
        ""
    };
    let stamp = match &plan.label {
        Some(label) => {
            parameters.push(("label".to_owned(), Value::String(label.clone())));
            format!(
                "{{ modified: $now, version_id: $label, notify: $notify, op: $op{replication} }}"
            )
        }
        None => format!("{{ modified: $now, notify: $notify, op: $op{replication} }}"),
    };
    let statement = match guard {
        Guard::None => {
            format!("UPSERT objects:$id = object::merge($object, {stamp}) RETURN AFTER;")
        }
        Guard::Absent => {
            format!("CREATE objects:$id = object::merge($object, {stamp}) RETURN AFTER;")
        }
        Guard::Etag(etag) => {
            parameters.push(("etag".to_owned(), Value::String(etag.clone())));
            format!(
                "UPDATE objects:$id = object::merge($object, {stamp}) \
                 WHERE etag = $etag AND incarnation = $incarnation RETURN AFTER;"
            )
        }
        Guard::Incarnation(stale) => {
            parameters.push(("stale".to_owned(), Value::Uuid(*stale)));
            format!(
                "UPDATE objects:$id = object::merge($object, {stamp}) \
                 WHERE incarnation = $stale RETURN AFTER;"
            )
        }
    };
    let changes = versions::statements(bucket, key, incarnation, plan, true)?;
    let versioned = changes.script;
    let commit = versions::commit(plan.local);
    parameters.extend(changes.parameters);
    // A data object commits together with the removal of its pending `reclaims` entry, and only while that entry exists
    // unmarked — the `UPDATE` refuses, and the whole transaction with it, once a reclaimer has taken the file. The
    // `supersede` event queues whatever data the record held before, in the same transaction. The write's answer
    // follows `BEGIN` and the `LET`; the version statements come last.
    let (script, answer_at) = match &object.content {
        // A completed upload: the upload must still exist (the `UPDATE` refuses the whole transaction once an
        // abort or another completion took it), the unlisted parts go (their event queues their files), and the
        // upload ends — all in the commit that makes the object visible.
        Content::Parts(multipart) => {
            let listed = multipart
                .parts
                .iter()
                .map(|part| Value::from(i64::from(part.number)))
                .collect();
            parameters.push(("upload".to_owned(), Value::Uuid(multipart.upload)));
            parameters.push(("listed".to_owned(), Value::Array(listed)));
            (
                format!(
                    "BEGIN; LET $now = time::now(); UPDATE uploads:$upload SET last_part = $now; \
                     {statement} \
                     DELETE FROM parts WHERE upload = $upload AND NOT (number IN $listed) LIMIT ALL; \
                     DELETE uploads:$upload; {versioned}{commit}"
                ),
                3,
            )
        }
        Content::Data(data) => {
            parameters.push(("data".to_owned(), Value::Uuid(*data)));
            (
                format!(
                    "BEGIN; LET $now = time::now(); {statement} \
                     UPDATE reclaims:$data SET data = $data WHERE reclaiming = NONE; \
                     DELETE reclaims:$data; {versioned}{commit}"
                ),
                2,
            )
        }
    };
    match pool.run(&script, parameters).await {
        Ok(answers) => match first_record(answers.into_iter().nth(answer_at))? {
            Some(value) => Ok(Wrote::Record(value)),
            None => Err(Error::Malformed {
                record: "object",
                reason: "a write answered no record",
            }),
        },
        Err(error) if is_lost_or_refused(&error) => Ok(Wrote::Refused),
        Err(error) => Err(error.into()),
    }
}
