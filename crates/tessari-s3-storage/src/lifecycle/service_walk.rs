//! One bucket's rules applied (S3 ref 08 M7): its versions walked in key order, each key's versions judged together,
//! then its open uploads. Every action is logged with the rule that took it.

use tessari_s3_constants::{LIST_MAX_KEYS, LIST_UPLOADS_MAX};
use tessari_s3_core::lifecycle::Rule;
use tessari_s3_core::objects::upload_listing::{UploadEntry, UploadSpec};
use tessari_s3_types::{BucketName, ObjectKey, Timestamp, UploadId};

use super::model::Applied;
use super::rules::{current_expires, elapsed, lone_marker_goes};
use super::service::LifecycleService;
use crate::multipart::UploadsListed;
use crate::objects::{ListedVersion, VersionKind, VersionRemoved, VersionsListed};
use crate::{Error, Result};

impl LifecycleService {
    /// Applies `rules` to `bucket` at `now`.
    pub(super) async fn apply(
        &self,
        bucket: &BucketName,
        rules: &[Rule],
        now: Timestamp,
        applied: &mut Applied,
    ) -> Result<()> {
        let enabled: Vec<&Rule> = rules.iter().filter(|rule| rule.enabled).collect();
        self.walk_versions(bucket, &enabled, now, applied).await?;
        self.abort_uploads(bucket, &enabled, now, applied).await
    }

    /// Walks every version of `bucket`. A key is judged once all its versions are read, and the walk resumes after
    /// the last version read, which no action has touched; a key read again after a version vanished under it skips
    /// what it already holds.
    async fn walk_versions(
        &self,
        bucket: &BucketName,
        rules: &[&Rule],
        now: Timestamp,
        applied: &mut Applied,
    ) -> Result<()> {
        let mut key_marker: Option<String> = None;
        let mut id_marker: Option<String> = None;
        let mut group_key = String::new();
        let mut group: Vec<ListedVersion> = Vec::new();
        loop {
            let spec = UploadSpec {
                prefix: String::new(),
                delimiter: None,
                max_uploads: LIST_MAX_KEYS,
                key_marker: key_marker.clone(),
            };
            let page = match self
                .objects
                .list_versions(bucket, spec, id_marker.as_deref())
                .await?
            {
                VersionsListed::Page(page) => page,
                VersionsListed::NoSuchBucket => return Ok(()),
                VersionsListed::MalformedMarker => {
                    return Err(Error::Malformed {
                        record: "version",
                        reason: "version id",
                    });
                }
            };
            let mut last = None;
            for entry in page.entries {
                let UploadEntry::Upload { key, item } = entry else {
                    continue;
                };
                last = Some((key.clone(), item.version_id.clone()));
                if key != group_key {
                    self.judge(bucket, &group_key, &group, rules, now, applied)
                        .await?;
                    group_key = key;
                    group.clear();
                }
                if group.last().is_some_and(|held| item.at <= held.at) {
                    continue;
                }
                group.push(item);
            }
            match last {
                Some((key, version)) if page.truncated => {
                    key_marker = Some(key);
                    id_marker = Some(version);
                }
                _ => break,
            }
        }
        self.judge(bucket, &group_key, &group, rules, now, applied)
            .await
    }

    /// Judges one key's versions, newest first, under the rules whose prefix it starts with.
    async fn judge(
        &self,
        bucket: &BucketName,
        key: &str,
        versions: &[ListedVersion],
        rules: &[&Rule],
        now: Timestamp,
        applied: &mut Applied,
    ) -> Result<()> {
        let Some(current) = versions.first() else {
            return Ok(());
        };
        let matching: Vec<&Rule> = rules
            .iter()
            .copied()
            .filter(|rule| key.starts_with(&rule.prefix))
            .collect();
        if matching.is_empty() {
            return Ok(());
        }
        let object_key = ObjectKey::new(key).map_err(|_| Error::Malformed {
            record: "version",
            reason: "key",
        })?;
        let mut removed = vec![false; versions.len()];
        // Noncurrent first: version i became noncurrent when version i − 1 was written.
        for rule in &matching {
            let Some(noncurrent) = &rule.noncurrent else {
                continue;
            };
            let keep = noncurrent
                .keep
                .map_or(0, |keep| usize::try_from(keep).unwrap_or(usize::MAX));
            for (index, pair) in versions.windows(2).enumerate() {
                let ordinal = index.saturating_add(1);
                if removed[ordinal]
                    || ordinal <= keep
                    || !elapsed(pair[0].modified, noncurrent.days, now)
                {
                    continue;
                }
                if self.remove(bucket, &object_key, &pair[1], &rule.id).await? {
                    removed[ordinal] = true;
                    applied.noncurrent = applied.noncurrent.saturating_add(1);
                }
            }
        }
        let alone = removed.iter().skip(1).all(|gone| *gone);
        match &current.kind {
            VersionKind::Object { etag, .. } => {
                let rule = matching
                    .iter()
                    .find(|rule| current_expires(rule, current.modified, now));
                if let Some(rule) = rule
                    && self
                        .objects
                        .expire_current(bucket, &object_key, etag, current.modified)
                        .await?
                {
                    tracing::info!(bucket = bucket.as_str(), key, version = %current.version_id, rule = %rule.id, "lifecycle expired the current version");
                    applied.expired = applied.expired.saturating_add(1);
                }
            }
            VersionKind::Marker if alone => {
                let rule = matching
                    .iter()
                    .find(|rule| lone_marker_goes(rule, current.modified, now));
                if let Some(rule) = rule
                    && self.remove(bucket, &object_key, current, &rule.id).await?
                {
                    applied.markers = applied.markers.saturating_add(1);
                }
            }
            VersionKind::Marker | VersionKind::Place => {}
        }
        Ok(())
    }

    /// Removes one version by id; `false` when it was already gone.
    async fn remove(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: &ListedVersion,
        rule: &str,
    ) -> Result<bool> {
        match self
            .objects
            .expire_version(bucket, key, &version.version_id)
            .await?
        {
            VersionRemoved::Done { marker } => {
                tracing::info!(bucket = bucket.as_str(), key = key.as_str(), version = %version.version_id, marker, rule, "lifecycle removed a version");
                Ok(true)
            }
            VersionRemoved::Absent | VersionRemoved::NoSuchBucket => Ok(false),
            VersionRemoved::Malformed => Err(Error::Malformed {
                record: "version",
                reason: "version id",
            }),
        }
    }

    /// Aborts the open uploads under each rule's prefix that were started its `DaysAfterInitiation` ago. They are
    /// collected first and aborted after the walk, so the walk never resumes behind an upload it removed.
    async fn abort_uploads(
        &self,
        bucket: &BucketName,
        rules: &[&Rule],
        now: Timestamp,
        applied: &mut Applied,
    ) -> Result<()> {
        let mut due: Vec<(UploadId, String)> = Vec::new();
        for rule in rules {
            let Some(days) = rule.abort_after_days else {
                continue;
            };
            let mut key_marker: Option<String> = None;
            let mut upload_marker = None;
            loop {
                let spec = UploadSpec {
                    prefix: rule.prefix.clone(),
                    delimiter: None,
                    max_uploads: LIST_UPLOADS_MAX,
                    key_marker: key_marker.clone(),
                };
                let page = match self
                    .multipart
                    .list_uploads(bucket, spec, upload_marker)
                    .await?
                {
                    UploadsListed::Page(page) => page,
                    UploadsListed::NoSuchBucket => return Ok(()),
                };
                let mut last = None;
                for entry in page.entries {
                    let UploadEntry::Upload { key, item } = entry else {
                        continue;
                    };
                    last = Some((key, item.id, item.initiated));
                    if elapsed(item.initiated, days, now)
                        && !due.iter().any(|(id, _)| *id == item.id)
                    {
                        due.push((item.id, rule.id.clone()));
                    }
                }
                match last {
                    Some((key, id, initiated)) if page.truncated => {
                        key_marker = Some(key);
                        upload_marker = Some((id, Some(initiated)));
                    }
                    _ => break,
                }
            }
        }
        for (id, rule) in due {
            self.multipart.abort(id).await?;
            tracing::info!(bucket = bucket.as_str(), upload = ?id, rule, "lifecycle aborted an incomplete upload");
            applied.aborted = applied.aborted.saturating_add(1);
        }
        Ok(())
    }
}
