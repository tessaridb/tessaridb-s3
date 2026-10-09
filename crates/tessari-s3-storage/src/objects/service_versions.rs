//! Which version records a write or a delete changes, by the bucket's versioning state (ADR-0010, S3 ref 08 M2-M3),
//! and the removal of one key with them. A plan is read from the key's head and committed with the key's `objects`
//! record, so a writer of the same key that commits first makes this one's commit refuse; the plan is then read again,
//! a bounded number of times.

use tessari_s3_constants::VERSION_WRITE_ATTEMPTS;
use tessari_s3_core::versioning::Versioning;
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::repository::{Head, NewVersion, ObjectRepository, VersionPlan};
use super::super::tessaridb::TessariObjects;
use super::super::version_id::{NULL, named};
use crate::{Error, Result};

/// A random nonce for a new version id.
fn nonce() -> Result<u64> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).map_err(|_| Error::Randomness)?;
    Ok(u64::from_be_bytes(bytes))
}

/// The version changes for a write (`delete` false) or a delete of the key whose head is `head`.
///
/// # Errors
/// [`Error::Randomness`] when no nonce can be drawn; [`Error::Malformed`] when the key's sequence is exhausted.
pub(super) fn plan(head: &Head, delete: bool) -> Result<VersionPlan> {
    let seq = match head.newest {
        None => 1,
        Some(newest) => newest.checked_add(1).ok_or(Error::Malformed {
            record: "versions",
            reason: "a key's version sequence is exhausted",
        })?,
    };
    let null = || NewVersion {
        seq,
        version_id: NULL.to_owned(),
    };
    let (create, drop_null, label) = match head.state {
        Some(Versioning::Enabled) => {
            let version_id = named(seq, nonce()?);
            let label = Some(version_id.clone());
            (Some(NewVersion { seq, version_id }), None, label)
        }
        // Writes and deletes replace the null version; versions written while enabled stay.
        Some(Versioning::Suspended) => (Some(null()), head.null, Some(NULL.to_owned())),
        // Never versioned: the key's one version is its null version, and a delete leaves it none.
        None => ((!delete).then(null), head.null, None),
    };
    Ok(VersionPlan {
        drop_null,
        create,
        label,
    })
}

/// Removes the current object at `bucket/key` with the version changes its bucket's state asks for, retrying while a
/// concurrent writer of the key keeps committing first.
///
/// # Errors
/// The metadata store's refusal or outage; [`Error::Contended`] when every attempt lost to another writer.
pub(super) async fn remove_key(
    repository: &TessariObjects,
    bucket: &BucketName,
    key: &ObjectKey,
    incarnation: [u8; 16],
) -> Result<()> {
    for _ in 0..VERSION_WRITE_ATTEMPTS {
        let head = repository.head(bucket, key).await?;
        let plan = plan(&head, true)?;
        if repository.remove(bucket, key, incarnation, &plan).await? {
            return Ok(());
        }
    }
    Err(Error::Contended)
}

#[cfg(test)]
mod tests {
    use tessari_s3_core::versioning::Versioning;

    use super::plan;
    use crate::objects::repository::Head;

    fn head(state: Option<Versioning>, newest: Option<u64>, null: Option<u64>) -> Head {
        Head {
            state,
            newest,
            null,
        }
    }

    #[test]
    fn an_enabled_write_adds_a_new_version_after_the_newest_and_keeps_the_null() {
        let made = plan(&head(Some(Versioning::Enabled), Some(4), Some(2)), false).expect("a plan");
        let created = made.create.expect("a version");
        assert_eq!((made.drop_null, created.seq), (None, 5));
        assert_eq!(created.version_id.len(), 32);
        assert_eq!(made.label, Some(created.version_id));
    }

    #[test]
    fn a_suspended_write_or_delete_replaces_only_the_null_version() {
        for delete in [false, true] {
            let made =
                plan(&head(Some(Versioning::Suspended), Some(4), Some(2)), delete).expect("a plan");
            let created = made.create.expect("a null version");
            assert_eq!(
                (made.drop_null, created.seq, created.version_id.as_str()),
                (Some(2), 5, "null")
            );
            assert_eq!(made.label.as_deref(), Some("null"));
        }
    }

    #[test]
    fn a_never_versioned_key_keeps_one_null_version_and_a_delete_leaves_none() {
        let write = plan(&head(None, Some(1), Some(1)), false).expect("a plan");
        assert_eq!((write.drop_null, write.label.clone()), (Some(1), None));
        assert_eq!(
            write
                .create
                .map(|version| (version.seq, version.version_id)),
            Some((2, "null".to_owned()))
        );
        let delete = plan(&head(None, Some(1), Some(1)), true).expect("a plan");
        assert_eq!((delete.drop_null, delete.create), (Some(1), None));
        let first = plan(&head(None, None, None), false).expect("a plan");
        assert_eq!(first.create.map(|version| version.seq), Some(1));
    }

    #[test]
    fn an_enabled_delete_adds_a_marker_after_the_newest() {
        let made = plan(&head(Some(Versioning::Enabled), Some(3), None), true).expect("a plan");
        assert_eq!(made.create.map(|version| version.seq), Some(4));
    }
}
