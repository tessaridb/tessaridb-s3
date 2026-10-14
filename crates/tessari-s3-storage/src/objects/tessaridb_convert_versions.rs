//! The statements that convert a store written before ADR-0010: each object a build without versions wrote becomes the
//! key's null version, the event that released a replaced object's parts goes (the versions own them now), and a
//! record marks the conversion done so a later start reads one record instead of the table.

use tessari_s3_infrastructure::tessaridb::{Answer, Value};

use super::super::version_id::{NULL, key_prefix, record_id};
use super::{TessariObjects, is_lost_or_refused};
use crate::answers::first_record;
use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "object",
        reason,
    }
}

/// The fields of an object a version copies; `modified` stays the object's own.
const FIELDS: &str = "bucket_name, key, incarnation, size, etag, modified, headers, metadata, checksums, data, upload, parts";

/// An object record as the conversion walks it.
pub(crate) struct Unconverted {
    /// The record id, `<bucket>/<key>`: the walk's cursor.
    pub(crate) id: String,
    /// The bytes every version of the key is addressed under.
    pub(crate) prefix: Vec<u8>,
    /// The bucket incarnation it was written under.
    pub(crate) incarnation: [u8; 16],
    /// The record's fields, as the version copies them.
    pub(crate) fields: Value,
}

fn unconverted(value: Value) -> Result<Unconverted> {
    let Value::Object(fields) = &value else {
        return Err(malformed("not an object"));
    };
    let text = |field: &'static str| match fields.get(field) {
        Some(Value::String(text)) => Ok(text.as_str()),
        _ => Err(malformed(field)),
    };
    let (bucket, key) = (text("bucket_name")?, text("key")?);
    let incarnation = match fields.get("incarnation") {
        Some(Value::Uuid(bytes)) => *bytes,
        _ => return Err(malformed("incarnation")),
    };
    // A record still holding its bytes (ADR-0008) points at neither; its version would point at nothing, so the bytes
    // move out first (`convert_inline`) and meeting one here refuses the start.
    if !matches!(fields.get("data"), Some(Value::Uuid(_)))
        && !matches!(fields.get("upload"), Some(Value::Uuid(_)))
    {
        return Err(malformed(
            "points at no data file or upload; its bytes are moved out before versions",
        ));
    }
    Ok(Unconverted {
        id: format!("{bucket}/{key}"),
        prefix: key_prefix(bucket, key),
        incarnation,
        fields: value,
    })
}

impl TessariObjects {
    /// Whether the conversion has run to the end on this store.
    pub(crate) async fn versions_converted(&self) -> Result<bool> {
        let answers = self
            .pool
            .run(
                "SELECT done FROM ONLY s3_conversions:'versions';",
                Vec::new(),
            )
            .await?;
        Ok(first_record(answers.into_iter().next())?.is_some())
    }

    /// Drops `parts_released` from `objects` when the table still carries it: a build without versions released a
    /// replaced object's parts there, and with versions a replaced object's parts belong to its version.
    pub(crate) async fn drop_parts_released(&self) -> Result<()> {
        let answers = self.pool.run("INFO FOR TABLE objects;", Vec::new()).await?;
        let Some(Answer::Value {
            value: Value::Object(info),
            ..
        }) = answers.into_iter().next()
        else {
            return Err(malformed("the table answered no description"));
        };
        let Some(Value::String(definition)) = info.get("definition") else {
            return Err(malformed("the table description has no definition"));
        };
        if definition.contains("DEFINE EVENT parts_released") {
            self.pool
                .run("DROP EVENT parts_released ON objects;", Vec::new())
                .await?;
        }
        Ok(())
    }

    /// At most `limit` object records after record `after` (exclusive; `""` starts the walk).
    pub(crate) async fn objects_after(
        &self,
        after: &str,
        limit: usize,
    ) -> Result<Vec<Unconverted>> {
        let parameters = vec![
            ("after".to_owned(), Value::String(after.to_owned())),
            ("limit".to_owned(), crate::params::count(limit)),
        ];
        let script = format!("SELECT {FIELDS} FROM objects AFTER objects:$after LIMIT $limit;");
        let answers = self.pool.run(&script, parameters).await?;
        match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => records
                .into_iter()
                .map(|(_, record)| unconverted(record))
                .collect(),
            _ => Ok(Vec::new()),
        }
    }

    /// Makes `object` its key's null version — the first version, so sequence 1 — while the record is still the one
    /// read and no version of the key exists: a key a versioned build has written already holds sequence 1, and the
    /// create collides. `false` when either did not hold, or a writer of the key committed meanwhile.
    pub(crate) async fn adopt_null_version(&self, object: &Unconverted) -> Result<bool> {
        let parameters = vec![
            ("id".to_owned(), Value::String(object.id.clone())),
            ("incarnation".to_owned(), Value::Uuid(object.incarnation)),
            ("object".to_owned(), object.fields.clone()),
            (
                "version".to_owned(),
                Value::Bytes(record_id(&object.prefix, 1)),
            ),
            ("key_at".to_owned(), Value::Bytes(object.prefix.clone())),
            ("null".to_owned(), Value::String(NULL.to_owned())),
        ];
        // The no-op update fences the record: a writer that replaced it since the read refuses this transaction.
        let script = "BEGIN; UPDATE objects:$id SET incarnation = $incarnation \
                      WHERE incarnation = $incarnation AND version_id = NONE; \
                      CREATE versions:$version = object::merge($object, \
                      { version_id: $null, seq: 1, marker: false }); \
                      CREATE version_nulls:$key_at = { seq: 1 }; COMMIT;";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if is_lost_or_refused(&error) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Records that the conversion ran to the end.
    pub(crate) async fn mark_versions_converted(&self) -> Result<()> {
        self.pool
            .run(
                "UPSERT s3_conversions:'versions' SET done = time::now();",
                Vec::new(),
            )
            .await?;
        Ok(())
    }
}
