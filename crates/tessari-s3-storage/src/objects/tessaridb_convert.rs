//! The statements that convert a store written before ADR-0008: object records still holding their bytes in an
//! `inline` field, and the column itself.

use tessari_s3_infrastructure::tessaridb::{Answer, Value};

use super::{TessariObjects, is_condition_refusal};
use crate::answers::all_records;
use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "object",
        reason,
    }
}

/// An object record that still holds its bytes.
pub(crate) struct InlineRecord {
    /// The record id, `<bucket>/<key>`.
    pub(crate) id: String,
    /// The bucket incarnation it was written under.
    pub(crate) incarnation: [u8; 16],
    /// The object's bytes.
    pub(crate) bytes: Vec<u8>,
}

fn inline_record(value: &Value) -> Result<InlineRecord> {
    let Value::Object(fields) = value else {
        return Err(malformed("not an object"));
    };
    let text = |field: &'static str| match fields.get(field) {
        Some(Value::String(text)) => Ok(text.as_str()),
        _ => Err(malformed(field)),
    };
    let incarnation = match fields.get("incarnation") {
        Some(Value::Uuid(bytes)) => *bytes,
        _ => return Err(malformed("incarnation")),
    };
    let bytes = match fields.get("inline") {
        Some(Value::Bytes(bytes)) => bytes.clone(),
        _ => return Err(malformed("inline")),
    };
    Ok(InlineRecord {
        id: format!("{}/{}", text("bucket_name")?, text("key")?),
        incarnation,
        bytes,
    })
}

impl TessariObjects {
    /// At most `limit` records still holding bytes. A store whose table never declared the column answers none: a
    /// condition on an undeclared field narrows rather than refuses.
    pub(crate) async fn inline_records(&self, limit: usize) -> Result<Vec<InlineRecord>> {
        // `LIMIT` takes a literal (0.33 refuses a parameter there); the limit is this server's own integer, never a
        // caller's value, and an integer formats to digits only.
        let script = format!(
            "SELECT bucket_name, key, incarnation, inline FROM objects WHERE inline != NONE LIMIT {limit};"
        );
        let answers = self.pool.run(&script, Vec::new()).await?;
        all_records(answers.into_iter().next(), "object")?
            .iter()
            .map(inline_record)
            .collect()
    }

    /// Points record `id` at data `data` and drops its bytes, only while it still holds them under `incarnation`, and
    /// takes the data id off the reclamation queue in the same transaction (refused once a reclaimer marked it).
    /// `false` when the record moved on or the data was taken.
    pub(crate) async fn adopt_data(
        &self,
        id: &str,
        incarnation: [u8; 16],
        data: [u8; 16],
    ) -> Result<bool> {
        let parameters = vec![
            ("id".to_owned(), Value::String(id.to_owned())),
            ("incarnation".to_owned(), Value::Uuid(incarnation)),
            ("data".to_owned(), Value::Uuid(data)),
        ];
        let script = "BEGIN; \
                      UPDATE objects:$id SET data = $data, inline = NONE \
                      WHERE inline != NONE AND incarnation = $incarnation; \
                      UPDATE reclaims:$data SET data = $data WHERE reclaiming = NONE; DELETE reclaims:$data; COMMIT;";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if is_condition_refusal(&error) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Whether the `objects` table still declares the `inline` column.
    pub(crate) async fn declares_inline(&self) -> Result<bool> {
        let answers = self.pool.run("INFO FOR TABLE objects;", Vec::new()).await?;
        let Some(Answer::Value {
            value: Value::Object(info),
            ..
        }) = answers.into_iter().next()
        else {
            return Err(malformed("the table answered no description"));
        };
        let Some(Value::Array(fields)) = info.get("fields") else {
            return Err(malformed("the table description lists no fields"));
        };
        Ok(fields.iter().any(|field| {
            matches!(field, Value::Object(declared)
                if declared.get("name") == Some(&Value::String("inline".to_owned())))
        }))
    }

    /// Drops the `inline` column; the store refuses while any record still holds bytes, and afterwards refuses any
    /// write that tries to add them.
    pub(crate) async fn drop_inline(&self) -> Result<()> {
        self.pool
            .run("DROP FIELD inline ON objects;", Vec::new())
            .await?;
        Ok(())
    }
}
