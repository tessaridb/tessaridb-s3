//! ListObjectVersions: the core lister fed batches of version records in listing order — keys in UTF-8 byte order,
//! each key's versions newest first (S3 ref 06 M5). Only the bucket's current incarnation is shown, and the walk stops
//! at the first record of another bucket.

use tessari_s3_constants::LIST_MAX_KEYS;
use tessari_s3_core::objects::upload_listing::{Marker, UploadAnchor, UploadLister, UploadSpec};
use tessari_s3_types::{BucketName, Timestamp};

use super::super::model::{ListedVersion, VersionKind, VersionsListed};
use super::super::version_id::{Naming, key_prefix, parse, record_id};
use super::ObjectService;
use crate::Result;

/// The record id a walk starts after to reach `anchor`. Ids are the bucket, `/`, the escaped key, `00 01`, then the
/// inverted sequence: no id equals a bare key prefix, every version of a key sorts below the prefix followed by eight
/// `FF` bytes, and no UTF-8 key carries an `FF` byte, so the escaped prefix followed by one `FF` is past every key
/// starting with it.
fn position(bucket: &BucketName, anchor: &UploadAnchor<ListedVersion>) -> Vec<u8> {
    match anchor {
        UploadAnchor::From(key) => key_prefix(bucket.as_str(), key),
        UploadAnchor::PastKey(key) => {
            let mut at = key_prefix(bucket.as_str(), key);
            at.extend_from_slice(&[0xFF; 8]);
            at
        }
        UploadAnchor::PastPrefix(prefix) => {
            let mut at = key_prefix(bucket.as_str(), prefix);
            // Drop the `00 01` terminator: the range is every key that merely starts with the prefix.
            at.truncate(at.len().saturating_sub(2));
            at.push(0xFF);
            at
        }
        UploadAnchor::After(version) => version.at.clone(),
    }
}

impl ObjectService {
    /// One ListObjectVersions page of `bucket` as `spec` asks for it, resumed after the version `id_marker` names
    /// (with `spec.key_marker`).
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn list_versions(
        &self,
        bucket: &BucketName,
        spec: UploadSpec,
        id_marker: Option<&str>,
    ) -> Result<VersionsListed> {
        let marker = match (spec.key_marker.as_deref(), id_marker) {
            (Some(key), Some(id)) => match parse(id) {
                None => return Ok(VersionsListed::MalformedMarker),
                Some(named) => self.place(bucket, key, named, id).await?,
            },
            _ => Marker::None,
        };
        // Resuming inside a key: the first version the walk meets is not that key's newest.
        let mut previous_key = match &marker {
            Marker::Found(_) => spec.key_marker.clone(),
            _ => None,
        };
        // One more than the page so the peek that decides IsTruncated usually costs no second read.
        let batch_size = spec.max_uploads.saturating_add(1).clamp(1, LIST_MAX_KEYS);
        let mut lister = UploadLister::new(spec, marker);
        let mut current = None;
        // Where to continue when a whole batch held nothing visible (an earlier incarnation's versions).
        let mut past_invisible: Option<Vec<u8>> = None;
        loop {
            let after = match (past_invisible.take(), lister.anchor()) {
                (Some(at), _) => at,
                (None, None) => break,
                (None, Some(anchor)) => position(bucket, anchor),
            };
            let batch = self
                .repository
                .version_batch(bucket, after, batch_size)
                .await?;
            let Some(now) = batch.bucket else {
                return Ok(VersionsListed::NoSuchBucket);
            };
            // The incarnation the listing started in holds for every page of it.
            let incarnation = *current.get_or_insert(now);
            let mut more = batch.full;
            let mut visible = Vec::with_capacity(batch.rows.len());
            let mut last = None;
            for row in batch.rows {
                if row.bucket != bucket.as_str() {
                    more = false;
                    break;
                }
                if row.incarnation != incarnation {
                    last = Some(row.at);
                    continue;
                }
                let latest = previous_key.as_deref() != Some(row.key.as_str());
                previous_key = Some(row.key.clone());
                let kind = match row.object {
                    Some((size, etag)) if !row.marker => VersionKind::Object { size, etag },
                    _ => VersionKind::Marker,
                };
                visible.push((
                    row.key,
                    ListedVersion {
                        at: row.at,
                        version_id: row.version_id,
                        latest,
                        modified: row.modified,
                        kind,
                    },
                ));
            }
            if visible.is_empty() && more {
                past_invisible = last;
                if past_invisible.is_none() {
                    break;
                }
                continue;
            }
            lister.feed(visible, more);
        }
        Ok(VersionsListed::Page(lister.finish()))
    }

    /// Where the version `id` (spelled so) of `key` sits in the listing. A numbered version's place is fixed by its
    /// sequence whether or not it still exists; the null version's is read, and gone when the key has none.
    async fn place(
        &self,
        bucket: &BucketName,
        key: &str,
        named: Naming,
        id: &str,
    ) -> Result<Marker<ListedVersion>> {
        let prefix = key_prefix(bucket.as_str(), key);
        let seq = match named {
            Naming::Seq(seq) => seq,
            Naming::Null => match self.repository.null_version_seq(&prefix).await? {
                Some(seq) => seq,
                None => return Ok(Marker::Gone),
            },
        };
        Ok(Marker::Found(ListedVersion {
            at: record_id(&prefix, seq),
            version_id: id.to_owned(),
            latest: false,
            modified: Timestamp {
                seconds: 0,
                nanos: 0,
            },
            kind: VersionKind::Place,
        }))
    }
}
