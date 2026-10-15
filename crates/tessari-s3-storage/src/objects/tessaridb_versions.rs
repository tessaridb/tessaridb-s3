//! Version records in TessariDB (ADR-0010): the read a version write starts from, and the statements that change a
//! key's versions inside the writing transaction. Every value is bound; record ids are bytes computed from the key.

use tessari_s3_core::versioning::Versioning;
use tessari_s3_infrastructure::tessaridb::{MetaPool, Number, Value};
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::repository::{Head, VersionPlan};
use super::super::version_id::{NULL, key_prefix, record_id};
use crate::answers::{bucket_read, first_record};
use crate::{Error, Result};

/// Where the statements of a plan bind their values; distinct from the object statement's own names.
pub(super) struct Statements {
    /// The statements, each ending in `;`.
    pub(super) script: String,
    /// Their bound values.
    pub(super) parameters: Vec<(String, Value)>,
}

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "versions",
        reason,
    }
}

fn stored_seq(seq: u64) -> Result<Value> {
    i64::try_from(seq)
        .map(Value::from)
        .map_err(|_| malformed("a sequence beyond i64"))
}

fn seq_field(record: Option<Value>) -> Result<Option<u64>> {
    let Some(Value::Object(fields)) = record else {
        return Ok(None);
    };
    match fields.get("seq") {
        Some(Value::Number(Number::Integer(seq))) => u64::try_from(*seq)
            .map(Some)
            .map_err(|_| malformed("a negative sequence")),
        _ => Err(malformed("seq")),
    }
}

/// The end of a transaction changing a key of a bucket with `local` acknowledgement or without.
pub(super) const fn commit(local: bool) -> &'static str {
    if local {
        "COMMIT ACKNOWLEDGE LOCAL MAJORITY;"
    } else {
        "COMMIT;"
    }
}

/// See `ObjectRepository::head`.
pub(super) async fn head(pool: &MetaPool, bucket: &BucketName, key: &ObjectKey) -> Result<Head> {
    let parameters = vec![
        (
            "bucket".to_owned(),
            Value::String(bucket.as_str().to_owned()),
        ),
        ("key".to_owned(), Value::String(key.as_str().to_owned())),
        (
            "prefix".to_owned(),
            Value::Bytes(key_prefix(bucket.as_str(), key.as_str())),
        ),
        (
            "object".to_owned(),
            Value::String(format!("{}/{}", bucket.as_str(), key.as_str())),
        ),
    ];
    // The record after the key's prefix is its newest version — or the first version of a later key, which the
    // bucket and key fields tell apart. The null version's sequence is kept at the key's own address in
    // `version_nulls`: an index cannot answer inside a transaction once anything in the store has committed since it
    // began, and this read must hold under concurrent writers.
    let script = format!(
        "BEGIN; {} SELECT bucket_name, key, seq FROM versions AFTER versions:$prefix LIMIT 1; \
         SELECT seq FROM ONLY version_nulls:$prefix; SELECT op FROM ONLY objects:$object; COMMIT;",
        bucket_read(pool, "versioning, notify, media, acknowledge, replication")
    );
    let mut answers = pool.run(&script, parameters).await?.into_iter();
    let _begin = answers.next();
    let bucket_record = first_record(answers.next())?;
    // A change is recorded for whoever reads `object_events`: the notification destinations and the media workers.
    let notifies = matches!(&bucket_record, Some(Value::Object(fields))
        if ["notify", "media"].iter().any(|field| !matches!(fields.get(*field), None | Some(Value::None))));
    let local = matches!(&bucket_record, Some(Value::Object(fields))
        if fields.get("acknowledge") == Some(&Value::String(crate::buckets::LOCAL.to_owned())));
    let replication = match &bucket_record {
        Some(Value::Object(fields)) => match fields.get("replication") {
            None | Some(Value::None) => None,
            Some(stored) => Some(crate::buckets::replication::from_value(stored)?),
        },
        _ => None,
    };
    let state = match bucket_record {
        Some(Value::Object(fields)) => match fields.get("versioning") {
            None | Some(Value::None) => None,
            Some(Value::String(state)) => {
                Some(Versioning::parse(state).ok_or(malformed("a bucket's versioning state"))?)
            }
            Some(_) => return Err(malformed("a bucket's versioning state")),
        },
        _ => None,
    };
    let newest = match first_record(answers.next())? {
        Some(Value::Object(fields))
            if fields.get("bucket_name") == Some(&Value::String(bucket.as_str().to_owned()))
                && fields.get("key") == Some(&Value::String(key.as_str().to_owned())) =>
        {
            seq_field(Some(Value::Object(fields)))?
        }
        _ => None,
    };
    let null = seq_field(first_record(answers.next())?)?;
    let current = first_record(answers.next())?.is_some();
    Ok(Head {
        local,
        state,
        newest,
        null,
        notifies,
        current,
        replication,
    })
}

/// The statements that carry out `plan` for the key, writing `$now` as the new version's time. A new object version
/// is the bound `$object` with its version fields merged in; a delete marker carries no content.
pub(super) fn statements(
    bucket: &BucketName,
    key: &ObjectKey,
    incarnation: [u8; 16],
    plan: &VersionPlan,
    with_object: bool,
) -> Result<Statements> {
    let prefix = key_prefix(bucket.as_str(), key.as_str());
    let mut script = String::new();
    let mut parameters = Vec::new();
    let null_moves = plan.drop_null.is_some()
        || plan
            .create
            .as_ref()
            .is_some_and(|version| version.version_id == NULL);
    if null_moves {
        parameters.push(("v_key_at".to_owned(), Value::Bytes(prefix.clone())));
    }
    if let Some(seq) = plan.drop_null {
        script.push_str("DELETE versions:$v_null; DELETE version_nulls:$v_key_at; ");
        parameters.push(("v_null".to_owned(), Value::Bytes(record_id(&prefix, seq))));
    }
    if let Some(version) = &plan.create {
        // A version to replicate is stamped `PENDING` with its destination, which the store's `replication_queued`
        // event queues in this commit; a replica is stamped `REPLICA` and has no destination.
        let mut fields = "version_id: $v_version_id, seq: $v_seq, modified: $now".to_owned();
        if let Some(status) = plan.replication_status() {
            fields.push_str(", replication_status: $v_replication_status");
            parameters.push((
                "v_replication_status".to_owned(),
                Value::String(status.to_owned()),
            ));
        }
        if let Some(destination) = &plan.replicate {
            fields.push_str(", replicate_to: $v_replicate_to");
            parameters.push((
                "v_replicate_to".to_owned(),
                Value::String(destination.clone()),
            ));
        }
        // A null version's sequence is kept at the key's own address, where `head` reads it by identity.
        if version.version_id == NULL {
            script.push_str("CREATE version_nulls:$v_key_at = { seq: $v_seq }; ");
        }
        if with_object {
            script.push_str(&format!(
                "CREATE versions:$v_version = object::merge($object, {{ {fields}, marker: false }}); "
            ));
        } else {
            script.push_str(&format!(
                "CREATE versions:$v_version = {{ bucket_name: $v_bucket, key: $v_key, \
                 incarnation: $v_incarnation, {fields}, marker: true }}; "
            ));
            parameters.push((
                "v_bucket".to_owned(),
                Value::String(bucket.as_str().to_owned()),
            ));
            parameters.push(("v_key".to_owned(), Value::String(key.as_str().to_owned())));
            parameters.push(("v_incarnation".to_owned(), Value::Uuid(incarnation)));
        }
        parameters.push((
            "v_version".to_owned(),
            Value::Bytes(record_id(&prefix, version.seq)),
        ));
        parameters.push((
            "v_version_id".to_owned(),
            Value::String(version.version_id.clone()),
        ));
        parameters.push(("v_seq".to_owned(), stored_seq(version.seq)?));
    }
    Ok(Statements { script, parameters })
}
