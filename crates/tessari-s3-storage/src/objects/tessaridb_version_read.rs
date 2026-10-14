//! Reading one version by its id, and whether a key's newest version is a delete marker (S3 ref 08 M4). Every read is
//! by identity or a walk of the key's own ids.

use tessari_s3_infrastructure::tessaridb::{Answer, Value};
use tessari_s3_types::{BucketName, ObjectKey, Timestamp};

use super::super::entity::{read, read_marker};
use super::super::model::{Part, StoredObject};
use super::super::version_id::{Naming, key_prefix, record_id};
use super::version_delete::null_seq;
use super::{TessariObjects, parts};
use crate::Result;
use crate::answers::first_record;

/// A version as it was read, with the incarnation of the bucket it was written in.
pub(crate) enum Found {
    /// A version holding an object, and its record id (for its parts).
    Object {
        incarnation: [u8; 16],
        object: StoredObject,
        at: Vec<u8>,
    },
    /// A delete marker and when it was written.
    Marker {
        incarnation: [u8; 16],
        modified: Timestamp,
    },
}

fn is_marker(record: &Value) -> bool {
    matches!(record, Value::Object(fields) if fields.get("marker") == Some(&Value::Bool(true)))
}

impl TessariObjects {
    /// The version of `bucket/key` that `named` (spelled `version_id`) names; `None` when no version has that id.
    pub(crate) async fn version_found(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        named: Naming,
        version_id: &str,
    ) -> Result<Option<Found>> {
        let prefix = key_prefix(bucket.as_str(), key.as_str());
        let seq = match named {
            Naming::Seq(seq) => seq,
            Naming::Null => match null_seq(&self.pool, &prefix).await? {
                Some(seq) => seq,
                None => return Ok(None),
            },
        };
        let at = record_id(&prefix, seq);
        let answers = self
            .pool
            .run(
                "SELECT * FROM ONLY versions:$at;",
                vec![("at".to_owned(), Value::Bytes(at.clone()))],
            )
            .await?;
        let Some(record) = first_record(answers.into_iter().next())? else {
            return Ok(None);
        };
        // The record at that sequence is another version when the id's nonce differs.
        let Value::Object(fields) = &record else {
            return Ok(None);
        };
        if fields.get("version_id") != Some(&Value::String(version_id.to_owned())) {
            return Ok(None);
        }
        if is_marker(&record) {
            let (incarnation, modified) = read_marker(&record)?;
            return Ok(Some(Found::Marker {
                incarnation,
                modified,
            }));
        }
        let (incarnation, object) = read(&record)?;
        Ok(Some(Found::Object {
            incarnation,
            object,
            at,
        }))
    }

    /// The parts of the multipart version at record id `at`, if it still names `upload` after they are read.
    pub(crate) async fn version_parts(
        &self,
        at: Vec<u8>,
        upload: [u8; 16],
    ) -> Result<Option<Vec<Part>>> {
        parts::parts_of(&self.pool, "versions", Value::Bytes(at), upload).await
    }

    /// The incarnation and id of the key's newest version when it is a delete marker.
    pub(crate) async fn newest_marker(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
    ) -> Result<Option<([u8; 16], String)>> {
        let parameters = vec![(
            "prefix".to_owned(),
            Value::Bytes(key_prefix(bucket.as_str(), key.as_str())),
        )];
        // The record after the key's prefix is its newest version — or a later key's, which its fields tell apart.
        let answers = self
            .pool
            .run(
                "SELECT bucket_name, key, incarnation, modified, version_id, marker FROM versions \
                 AFTER versions:$prefix LIMIT 1;",
                parameters,
            )
            .await?;
        let record = match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => {
                records.into_iter().next().map(|(_, value)| value)
            }
            _ => None,
        };
        let Some(record) = record else {
            return Ok(None);
        };
        let Value::Object(fields) = &record else {
            return Ok(None);
        };
        let ours = fields.get("bucket_name") == Some(&Value::String(bucket.as_str().to_owned()))
            && fields.get("key") == Some(&Value::String(key.as_str().to_owned()));
        let Some(Value::String(id)) = fields
            .get("version_id")
            .filter(|_| ours && is_marker(&record))
        else {
            return Ok(None);
        };
        let (incarnation, _) = read_marker(&record)?;
        Ok(Some((incarnation, id.clone())))
    }
}
