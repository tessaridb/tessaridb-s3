//! Replicas another bucket sends (S3 ref 11 M4): a replica keeps the version id its source gave it, so it is stored
//! at that id's sequence; it is the current object only when that sequence is the key's newest, and an older one
//! lands as a noncurrent version. A replica is stamped `REPLICA` and never replicated again — the loop guard of a
//! two-way setup. Sending the same version twice writes nothing the second time.

use tessari_s3_constants::VERSION_WRITE_ATTEMPTS;
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::model::{Content, NewObject, Replicated};
use super::super::repository::{
    Guard, Head, NewVersion, ObjectRepository, Removal, VersionPlan, Wrote,
};
use super::super::version_id::{Naming, parse};
use super::ObjectService;
use crate::{Error, Result};

/// What a replica of the version named `version_id` (numbered `seq`) does to the key whose head is `head`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReplicaPlan {
    /// That version is already here: nothing to write.
    Kept,
    /// Another version holds that sequence here: the replica cannot be stored.
    Conflict,
    /// Store it with `plan`; `current` when it becomes the key's current version.
    Write {
        /// The version changes.
        plan: VersionPlan,
        /// Whether the replica is the key's newest version.
        current: bool,
    },
}

/// The plan for a replica of `version_id`, numbered `seq`, given the id already stored at that sequence.
pub(super) fn replica_plan(
    head: &Head,
    existing: Option<&str>,
    version_id: &str,
    seq: u64,
) -> ReplicaPlan {
    match existing {
        Some(stored) if stored == version_id => return ReplicaPlan::Kept,
        Some(_) => return ReplicaPlan::Conflict,
        None => {}
    }
    let current = head.newest.is_none_or(|newest| seq > newest);
    ReplicaPlan::Write {
        plan: VersionPlan {
            drop_null: None,
            create: Some(NewVersion {
                seq,
                version_id: version_id.to_owned(),
            }),
            label: Some(version_id.to_owned()),
            notify: head.notifies,
            current: head.current,
            removal: Removal::Request,
            local: head.local,
            replicate: None,
            replica: true,
            lock_default: head.lock_default,
        },
        current,
    }
}

impl ObjectService {
    /// Stores `object` as the replica of version `version_id` at `bucket/key`. A data file the replica does not end up
    /// holding is released.
    ///
    /// # Errors
    /// The metadata store's refusal or outage; [`Error::Contended`] when every attempt lost to another writer of
    /// the key.
    pub async fn put_replica(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        object: &NewObject,
        version_id: &str,
    ) -> Result<Replicated> {
        let outcome = self.replicate(bucket, key, Some(object), version_id).await;
        if let (Content::Data(id), false) =
            (&object.content, matches!(outcome, Ok(Replicated::Stored)))
        {
            self.release_logged(*id).await;
        }
        outcome
    }

    /// Stores the replica of delete marker `version_id` at `bucket/key`.
    ///
    /// # Errors
    /// As [`Self::put_replica`].
    pub async fn mark_replica(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version_id: &str,
    ) -> Result<Replicated> {
        self.replicate(bucket, key, None, version_id).await
    }

    async fn replicate(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        object: Option<&NewObject>,
        version_id: &str,
    ) -> Result<Replicated> {
        let Some(Naming::Seq(seq)) = parse(version_id) else {
            return Ok(Replicated::NotAVersionId);
        };
        let Some(incarnation) = self.repository.incarnation(bucket).await? else {
            return Ok(Replicated::NoSuchBucket);
        };
        for _ in 0..VERSION_WRITE_ATTEMPTS {
            let head = self.repository.head(bucket, key).await?;
            let existing = self.repository.version_id_at(bucket, key, seq).await?;
            let (plan, current) = match replica_plan(&head, existing.as_deref(), version_id, seq) {
                ReplicaPlan::Kept => return Ok(Replicated::Kept),
                ReplicaPlan::Conflict => return Ok(Replicated::Conflict),
                ReplicaPlan::Write { plan, current } => (plan, current),
            };
            // The newest replica writes or removes the current object as any write or delete does; an older one only
            // adds its version. A refusal means another writer of the key committed first: plan again.
            let stored = match (object, current) {
                (Some(object), true) => matches!(
                    self.repository
                        .write(bucket, key, incarnation, object, &Guard::None, &plan)
                        .await?,
                    Wrote::Record(_)
                ),
                (None, true) => {
                    self.repository
                        .remove(bucket, key, incarnation, &plan)
                        .await?
                }
                (object, false) => {
                    self.repository
                        .add_version(bucket, key, incarnation, object, &plan)
                        .await?
                }
            };
            if stored {
                return Ok(Replicated::Stored);
            }
        }
        Err(Error::Contended)
    }
}

#[cfg(test)]
mod tests {
    use tessari_s3_core::replication::{Configuration, Rule};
    use tessari_s3_core::versioning::Versioning;

    use super::{ReplicaPlan, replica_plan};
    use crate::objects::repository::Head;

    const ID: &str = "0000000000000005000000000000beef";

    /// A key whose newest version is `newest`, in a bucket that would itself replicate every key.
    fn head(newest: Option<u64>) -> Head {
        Head {
            state: Some(Versioning::Enabled),
            newest,
            null: None,
            notifies: true,
            current: newest.is_some(),
            local: false,
            replication: Some(Configuration {
                role: "r".to_owned(),
                rules: vec![Rule {
                    id: "all".to_owned(),
                    enabled: true,
                    priority: 1,
                    prefix: String::new(),
                    destination: "elsewhere".to_owned(),
                    delete_markers: true,
                }],
            }),
            lock_default: None,
        }
    }

    fn written(planned: ReplicaPlan) -> (crate::objects::repository::VersionPlan, bool) {
        match planned {
            ReplicaPlan::Write { plan, current } => (plan, current),
            other => panic!("a write, not {other:?}"),
        }
    }

    #[test]
    fn a_replica_keeps_its_id_at_its_sequence_and_is_never_sent_on() {
        let (plan, current) = written(replica_plan(&head(Some(4)), None, ID, 5));
        let created = plan.create.clone().expect("a version");
        assert_eq!((created.seq, created.version_id.as_str()), (5, ID));
        assert_eq!(plan.label.as_deref(), Some(ID));
        assert!(current, "sequence 5 is newer than 4");
        assert_eq!(plan.replication_status(), Some("REPLICA"));
        assert_eq!(
            plan.replicate, None,
            "even though the bucket's own rule covers the key"
        );
        assert_eq!(plan.drop_null, None);
        assert!(plan.notify, "a replica notifies like any write");
    }

    #[test]
    fn an_older_replica_lands_behind_the_current_version() {
        let (_, current) = written(replica_plan(&head(Some(9)), None, ID, 5));
        assert!(!current);
        let (_, first) = written(replica_plan(&head(None), None, ID, 5));
        assert!(first, "the key's only version is current");
    }

    #[test]
    fn the_same_version_sent_again_writes_nothing_and_another_at_its_sequence_conflicts() {
        assert_eq!(
            replica_plan(&head(Some(5)), Some(ID), ID, 5),
            ReplicaPlan::Kept
        );
        assert_eq!(
            replica_plan(
                &head(Some(5)),
                Some("00000000000000050000000000000001"),
                ID,
                5
            ),
            ReplicaPlan::Conflict
        );
    }
}
