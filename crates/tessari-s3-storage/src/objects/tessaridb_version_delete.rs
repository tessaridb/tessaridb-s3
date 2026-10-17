//! Deleting one version by its id (S3 ref 08 M3): the version record goes, and when it was the newest the next newest
//! becomes the current object — or the key has none when that is a delete marker or nothing. Every read is by identity
//! or a walk of the key's own ids, so it holds beside concurrent writers.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::repository::Removal;
use super::super::version_id::{NULL, Naming, key_prefix, record_id};
use super::{TessariObjects, id, is_lock_refusal, is_lost_or_refused, versions};
use crate::answers::{bucket_read, first_record};
use crate::{Error, Result};

/// A version's fields, as a read of it projects them.
/// Every field a version brings back when it becomes current again — its wrapped data key included, or a sealed
/// version promoted from here could not be opened.
const FIELDS: &str = "bucket_name, key, incarnation, size, etag, modified, headers, metadata, tags, checksums, data, \
                      upload, parts, version_id, seq, marker, sse_alg, sse_kek, sse_nonce, sse_key, sse_c_salt, \
                      sse_c_verifier";

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "versions",
        reason,
    }
}

/// What becomes current when the newest version goes.
pub(crate) enum Successor {
    /// A version holding an object: these fields become the key's `objects` record.
    Object(Value),
    /// A delete marker, or no version at all: the key has no current object.
    Nothing,
}

/// What Object Lock keeps a version under, judged by the metadata store's clock when it was read (ADR-0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kept {
    /// Nothing: no legal hold, and no retention or one that has passed.
    Free,
    /// A legal hold.
    Held,
    /// A GOVERNANCE retention that has not passed.
    Governance,
    /// A COMPLIANCE retention that has not passed.
    Compliance,
}

impl Kept {
    /// Whether it keeps the version from a removal that asks to bypass governance retention or not.
    pub(crate) const fn keeps(self, bypass: bool) -> bool {
        match self {
            Self::Free => false,
            Self::Held | Self::Compliance => true,
            Self::Governance => !bypass,
        }
    }
}

fn kept(answer: Option<Value>) -> Result<Kept> {
    let Some(Value::Object(fields)) = answer else {
        return Ok(Kept::Free);
    };
    if fields.get("held") == Some(&Value::Bool(true)) {
        return Ok(Kept::Held);
    }
    if fields.get("retained") != Some(&Value::Bool(true)) {
        return Ok(Kept::Free);
    }
    match fields.get("lock_mode") {
        Some(Value::String(mode)) if mode == "COMPLIANCE" => Ok(Kept::Compliance),
        Some(Value::String(mode)) if mode == "GOVERNANCE" => Ok(Kept::Governance),
        _ => Err(malformed("a retained version's lock mode")),
    }
}

/// The version a delete names, as it was read.
pub(crate) struct Target {
    seq: u64,
    version_id: String,
    /// Whether it is a delete marker.
    pub(crate) marker: bool,
    /// What Object Lock keeps it under.
    pub(crate) kept: Kept,
    /// What becomes current, when the target is the key's newest version.
    successor: Option<Successor>,
    /// Whether the bucket notifies, read with the target.
    notify: bool,
    /// Whether the bucket's writes are acknowledged by its region's majority alone.
    local: bool,
}

type Fields = std::collections::BTreeMap<String, Value>;

fn fields(value: Value) -> Result<Fields> {
    match value {
        Value::Object(fields) => Ok(fields),
        _ => Err(malformed("not an object")),
    }
}

fn seq(fields: &Fields) -> Result<u64> {
    match fields.get("seq") {
        Some(Value::Number(Number::Integer(seq))) => {
            u64::try_from(*seq).map_err(|_| malformed("a negative sequence"))
        }
        _ => Err(malformed("seq")),
    }
}

fn is_marker(fields: &Fields) -> bool {
    matches!(fields.get("marker"), Some(Value::Bool(true)))
}

/// The newest versions of the key the walk read, newest first, the walk's overrun into a later key dropped.
fn of_key(answer: Option<Answer>, bucket: &BucketName, key: &ObjectKey) -> Result<Vec<Fields>> {
    let records = match answer {
        Some(Answer::Records { records, .. }) => records,
        _ => Vec::new(),
    };
    let mut versions = Vec::new();
    for (_, record) in records {
        let fields = fields(record)?;
        let ours = fields.get("bucket_name") == Some(&Value::String(bucket.as_str().to_owned()))
            && fields.get("key") == Some(&Value::String(key.as_str().to_owned()));
        if ours {
            versions.push(fields);
        }
    }
    Ok(versions)
}

/// The sequence the key's null version sits at, if it has one.
pub(super) async fn null_seq(pool: &MetaPool, prefix: &[u8]) -> Result<Option<u64>> {
    let parameters = vec![("key_at".to_owned(), Value::Bytes(prefix.to_vec()))];
    let answers = pool
        .run("SELECT seq FROM ONLY version_nulls:$key_at;", parameters)
        .await?;
    first_record(answers.into_iter().next())?
        .map(|record| seq(&fields(record)?))
        .transpose()
}

impl TessariObjects {
    /// The version of `bucket/key` that `named` (spelled `version_id`) names, with what becomes current if it goes;
    /// `None` when no version of the key has that id.
    pub(crate) async fn version_target(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        named: Naming,
        version_id: &str,
    ) -> Result<Option<Target>> {
        let prefix = key_prefix(bucket.as_str(), key.as_str());
        let target_seq = match named {
            Naming::Seq(seq) => seq,
            Naming::Null => match null_seq(&self.pool, &prefix).await? {
                Some(seq) => seq,
                None => return Ok(None),
            },
        };
        let parameters = vec![
            (
                "target".to_owned(),
                Value::Bytes(record_id(&prefix, target_seq)),
            ),
            ("prefix".to_owned(), Value::Bytes(prefix)),
            (
                "bucket".to_owned(),
                Value::String(bucket.as_str().to_owned()),
            ),
        ];
        let script = format!(
            "BEGIN; SELECT {FIELDS} FROM ONLY versions:$target; \
             SELECT (legal_hold ?? false) AS held, (retain_until != NONE AND retain_until > time::now()) AS retained, \
             lock_mode FROM ONLY versions:$target; \
             SELECT {FIELDS} FROM versions AFTER versions:$prefix LIMIT 2; {} COMMIT;",
            bucket_read(&self.pool, "notify, acknowledge")
        );
        let mut answers = self.pool.run(&script, parameters).await?.into_iter();
        let _begin = answers.next();
        let Some(target) = first_record(answers.next())?.map(fields).transpose()? else {
            return Ok(None);
        };
        // The record at that sequence is another version when the id's nonce differs.
        if target.get("version_id") != Some(&Value::String(version_id.to_owned())) {
            return Ok(None);
        }
        let kept = kept(first_record(answers.next())?)?;
        let mut newest = of_key(answers.next(), bucket, key)?.into_iter();
        let bucket_record = first_record(answers.next())?;
        let notify = matches!(&bucket_record, Some(Value::Object(fields))
            if !matches!(fields.get("notify"), None | Some(Value::None)));
        let local = matches!(&bucket_record, Some(Value::Object(fields))
            if fields.get("acknowledge") == Some(&Value::String(crate::buckets::LOCAL.to_owned())));
        let successor = match newest.next() {
            Some(first) if seq(&first)? == target_seq => Some(match newest.next() {
                Some(next) if !is_marker(&next) => {
                    let mut restored = next;
                    restored.remove("seq");
                    restored.remove("marker");
                    Successor::Object(Value::Object(restored))
                }
                _ => Successor::Nothing,
            }),
            _ => None,
        };
        Ok(Some(Target {
            seq: target_seq,
            marker: is_marker(&target),
            kept,
            version_id: version_id.to_owned(),
            successor,
            notify,
            local,
        }))
    }

    /// Removes `target` as it was read, in one transaction; `false` when the key moved on since (the version is not
    /// the one read, or the current object changed), so the caller reads it again.
    pub(crate) async fn remove_version(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        target: &Target,
        removal: Removal,
        bypass: bool,
    ) -> Result<bool> {
        let prefix = key_prefix(bucket.as_str(), key.as_str());
        let mut parameters = vec![
            ("id".to_owned(), id(bucket, key)),
            (
                "target".to_owned(),
                Value::Bytes(record_id(&prefix, target.seq)),
            ),
            ("vid".to_owned(), Value::String(target.version_id.clone())),
            ("null".to_owned(), Value::String(NULL.to_owned())),
        ];
        // Sequences are reused once the newest version goes, so the record is removed only while it is the one read.
        // Where the bucket notifies, the fence also stamps the message the store's `version_removed` event appends:
        // its own name (never a generated identity, which concurrent removals would contend on) and its event.
        let stamp = if target.notify {
            parameters.push((
                "removal".to_owned(),
                Value::String(format!("v-{}", uuid::Uuid::new_v4())),
            ));
            parameters.push((
                "removal_event".to_owned(),
                Value::String(format!("s3:{}:Delete", removal.family())),
            ));
            ", removal = $removal, removal_event = $removal_event"
        } else {
            ""
        };
        // A GOVERNANCE retention lets the removal through only with the bypass flag set in this same transaction,
        // which the store's `locked_release` event reads (ADR-0016); the record goes with the flag on it.
        let bypassing = if bypass && target.kept == Kept::Governance {
            "UPDATE versions:$target SET lock_bypass = true; "
        } else {
            ""
        };
        let mut script = format!(
            "BEGIN; {bypassing}UPDATE versions:$target SET version_id = $vid{stamp} WHERE version_id = $vid; \
             DELETE versions:$target; ",
        );
        if target.version_id == NULL {
            script.push_str("DELETE version_nulls:$key_at; ");
            parameters.push(("key_at".to_owned(), Value::Bytes(prefix)));
        }
        match (&target.successor, target.marker) {
            // Not the newest: the current object stays as it is.
            (None, _) => {}
            // The current object is this version (a never-versioned object carries no id: its version is `null`) —
            // fenced on that, then replaced by the next newest or removed. The removal is the version's own event, so
            // the fence clears `notify` and the object's `object_removed` stays silent.
            (Some(successor), false) => {
                script.push_str(
                    "UPDATE objects:$id SET version_id = $vid, notify = false \
                     WHERE (version_id ?? $null) = $vid; ",
                );
                match successor {
                    Successor::Object(restored) => {
                        script.push_str("UPDATE objects:$id = $restored; ");
                        parameters.push(("restored".to_owned(), restored.clone()));
                    }
                    Successor::Nothing => script.push_str("DELETE objects:$id; "),
                }
            }
            // The current version is this marker, so the key has no object: the next newest becomes one, and a writer
            // that made one meanwhile makes the create collide.
            (Some(Successor::Object(restored)), true) => {
                script.push_str("CREATE objects:$id = $restored; ");
                parameters.push(("restored".to_owned(), restored.clone()));
            }
            (Some(Successor::Nothing), true) => {}
        }
        script.push_str(versions::commit(target.local));
        match self.pool.run(&script, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if is_lost_or_refused(&error) => Ok(false),
            // Held or retained since it was read: reading it again finds what keeps it.
            Err(error) if is_lock_refusal(&error) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}
