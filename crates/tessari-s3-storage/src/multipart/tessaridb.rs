//! Multipart uploads in TessariDB: `uploads:<upload id>` and `parts:'<upload id hex>/<00001>'`, so a part's record id
//! orders by upload and then by number and ListParts walks it with `AFTER`. Every value is bound.

use std::collections::BTreeMap;

use tessari_s3_constants::MULTIPART_MAX_PARTS;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, RefusalClass, Value};
use tessari_s3_types::{BucketName, ObjectKey, PartNumber, Timestamp, UploadId};

use super::entity::complete_row;
use super::model::{NewPart, NewUpload, StoredPart};
use super::repository::{MultipartRepository, PartRow, UploadRecord};
use crate::answers::{first_record, incarnation_of};
use crate::{Error, Result};

/// The TessariDB multipart repository.
#[derive(Clone)]
pub struct TessariMultipart {
    pool: MetaPool,
}

impl TessariMultipart {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

pub(super) fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "part",
        reason,
    }
}

/// A part's record id: the upload in hex, then the number zero-padded, so ids order by upload and then by number.
fn part_id(upload: UploadId, number: u16) -> Value {
    Value::String(format!("{}/{number:05}", upload.hex()))
}

fn strings(map: &BTreeMap<String, String>) -> Value {
    Value::Object(
        map.iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect(),
    )
}

pub(super) fn string_map(value: Option<&Value>) -> Result<BTreeMap<String, String>> {
    let Some(Value::Object(map)) = value else {
        return Err(malformed("checksums"));
    };
    map.iter()
        .map(|(k, v)| match v {
            Value::String(text) => Ok((k.clone(), text.clone())),
            _ => Err(malformed("checksums")),
        })
        .collect()
}

/// A part row of ListParts, with the upload it belongs to.
fn part_row(value: &Value) -> Result<([u8; 16], StoredPart)> {
    let Value::Object(fields) = value else {
        return Err(malformed("not an object"));
    };
    let integer = |field: &'static str| match fields.get(field) {
        Some(Value::Number(Number::Integer(n))) => Ok(*n),
        _ => Err(malformed(field)),
    };
    let upload = match fields.get("upload") {
        Some(Value::Uuid(id)) => *id,
        _ => return Err(malformed("upload")),
    };
    let number = u16::try_from(integer("number")?)
        .ok()
        .and_then(|n| PartNumber::new(n).ok())
        .ok_or_else(|| malformed("number"))?;
    let part = StoredPart {
        number,
        size: u64::try_from(integer("size")?).map_err(|_| malformed("size"))?,
        etag: match fields.get("etag") {
            Some(Value::String(etag)) => etag.clone(),
            _ => return Err(malformed("etag")),
        },
        modified: match fields.get("modified") {
            Some(Value::Datetime { seconds, nanos }) => Timestamp {
                seconds: *seconds,
                nanos: *nanos,
            },
            _ => return Err(malformed("modified")),
        },
        checksums: string_map(fields.get("checksums"))?,
    };
    Ok((upload, part))
}

impl MultipartRepository for TessariMultipart {
    async fn incarnation(&self, bucket: &BucketName) -> Result<Option<[u8; 16]>> {
        let parameters = vec![(
            "bucket".to_owned(),
            Value::String(bucket.as_str().to_owned()),
        )];
        let answers = self
            .pool
            .run("SELECT incarnation FROM ONLY buckets:$bucket;", parameters)
            .await?;
        incarnation_of(first_record(answers.into_iter().next())?)
    }

    async fn create(
        &self,
        id: UploadId,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        upload: &NewUpload,
    ) -> Result<()> {
        let mut record: BTreeMap<String, Value> = [
            ("bucket_name", Value::String(bucket.as_str().to_owned())),
            ("key", Value::String(key.as_str().to_owned())),
            ("incarnation", Value::Uuid(incarnation)),
            ("headers", strings(&upload.headers)),
            ("metadata", strings(&upload.metadata)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        for (field, value) in [
            ("checksum_algorithm", &upload.checksum_algorithm),
            ("checksum_type", &upload.checksum_type),
        ] {
            if let Some(value) = value {
                record.insert(field.to_owned(), Value::String(value.clone()));
            }
        }
        let parameters = vec![
            ("upload".to_owned(), Value::Uuid(id.bytes())),
            ("record".to_owned(), Value::Object(record)),
        ];
        self.pool
            .run(
                "CREATE uploads:$upload = object::merge($record, { initiated: time::now() });",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn upload(&self, id: UploadId) -> Result<Option<UploadRecord>> {
        let parameters = vec![("upload".to_owned(), Value::Uuid(id.bytes()))];
        let answers = self
            .pool
            .run(
                "SELECT bucket_name, key, incarnation, headers, metadata, checksum_algorithm, checksum_type \
                 FROM ONLY uploads:$upload;",
                parameters,
            )
            .await?;
        let Some(Value::Object(fields)) = first_record(answers.into_iter().next())? else {
            return Ok(None);
        };
        let text = |field: &'static str| match fields.get(field) {
            Some(Value::String(text)) => Ok(text.clone()),
            _ => Err(Error::Malformed {
                record: "upload",
                reason: field,
            }),
        };
        let optional = |field: &'static str| match fields.get(field) {
            Some(Value::String(text)) => Some(text.clone()),
            _ => None,
        };
        Ok(Some(UploadRecord {
            bucket: text("bucket_name")?,
            key: text("key")?,
            declared: NewUpload {
                headers: string_map(fields.get("headers"))?,
                metadata: string_map(fields.get("metadata"))?,
                checksum_algorithm: optional("checksum_algorithm"),
                checksum_type: optional("checksum_type"),
            },
            incarnation: match fields.get("incarnation") {
                Some(Value::Uuid(bytes)) => *bytes,
                _ => {
                    return Err(Error::Malformed {
                        record: "upload",
                        reason: "incarnation",
                    });
                }
            },
        }))
    }

    async fn write_part(&self, id: UploadId, number: PartNumber, part: &NewPart) -> Result<bool> {
        let size = i64::try_from(part.size).map_err(|_| malformed("size beyond i64"))?;
        let record: BTreeMap<String, Value> = [
            ("upload", Value::Uuid(id.bytes())),
            ("number", Value::from(i64::from(number.get()))),
            ("data", Value::Uuid(part.data)),
            ("size", Value::from(size)),
            ("etag", Value::String(part.etag.clone())),
            ("checksums", strings(&part.checksums)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let parameters = vec![
            ("upload".to_owned(), Value::Uuid(id.bytes())),
            ("id".to_owned(), part_id(id, number.get())),
            ("part".to_owned(), Value::Object(record)),
            ("data".to_owned(), Value::Uuid(part.data)),
        ];
        // The upload must still exist (an `UPDATE` of a missing record refuses the whole transaction), the part
        // replaces any earlier one of its number (`part_superseded` queues the earlier data), and the part's own data
        // leaves the reclamation queue only while no reclaimer has claimed it.
        let script = "BEGIN; UPDATE uploads:$upload SET last_part = time::now() RETURN AFTER; \
                      UPSERT parts:$id = object::merge($part, { modified: time::now() }); \
                      UPDATE gc:$data SET data = $data WHERE reclaiming = NONE; DELETE gc:$data; COMMIT;";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(true),
            Err(error)
                if error.is_class(RefusalClass::Conflict)
                    || error.is_class(RefusalClass::Invalid) =>
            {
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn parts(
        &self,
        id: UploadId,
        after: u16,
        limit: usize,
    ) -> Result<(Vec<StoredPart>, bool)> {
        let parameters = vec![("after".to_owned(), part_id(id, after))];
        // One more than the page decides IsTruncated. `LIMIT` takes a literal on 0.33.1; this is the server's own
        // integer, never a caller's value.
        let fetch = limit.saturating_add(1);
        let script = format!(
            "SELECT upload, number, size, etag, modified, checksums FROM parts AFTER parts:$after LIMIT {fetch};"
        );
        let answers = self.pool.run(&script, parameters).await?;
        let records = match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => records,
            _ => Vec::new(),
        };
        let mut parts = Vec::with_capacity(records.len());
        for (_, value) in &records {
            let (upload, part) = part_row(value)?;
            if upload != id.bytes() {
                break;
            }
            parts.push(part);
        }
        let more = parts.len() > limit;
        parts.truncate(limit);
        Ok((parts, more))
    }

    async fn all_parts(&self, id: UploadId) -> Result<Vec<PartRow>> {
        let parameters = vec![("upload".to_owned(), Value::Uuid(id.bytes()))];
        // `LIMIT` takes a literal; this is the server's own constant.
        let script = format!(
            "SELECT number, data, size, etag, checksums FROM parts WHERE upload = $upload \
             LIMIT {MULTIPART_MAX_PARTS} USING INDEX by_upload;"
        );
        let answers = self.pool.run(&script, parameters).await?;
        let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
            return Ok(Vec::new());
        };
        records
            .iter()
            .map(|(_, value)| complete_row(value))
            .collect()
    }

    async fn abort(&self, id: UploadId) -> Result<()> {
        let parameters = vec![("upload".to_owned(), Value::Uuid(id.bytes()))];
        self.pool
            .run(
                "BEGIN; DELETE uploads:$upload; DELETE FROM parts WHERE upload = $upload LIMIT ALL; COMMIT;",
                parameters,
            )
            .await?;
        Ok(())
    }
}
