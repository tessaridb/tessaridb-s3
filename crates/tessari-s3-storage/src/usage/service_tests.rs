use std::sync::Mutex;

use super::entity::{BucketUsageEntity, UsageEntity};
use super::repository::UsageRepository;
use super::service::refresh;
use crate::Result;

/// A repository whose claim is held by `holder`, recording what was stored.
struct Fake {
    holder: &'static str,
    measured: Vec<BucketUsageEntity>,
    stored: Mutex<Option<Vec<BucketUsageEntity>>>,
}

impl UsageRepository for Fake {
    async fn claim(&self, holder: &str) -> Result<bool> {
        Ok(holder == self.holder)
    }

    async fn measure(&self) -> Result<Vec<BucketUsageEntity>> {
        Ok(self.measured.clone())
    }

    async fn store(&self, buckets: &[BucketUsageEntity]) -> Result<()> {
        *self.stored.lock().expect("unpoisoned") = Some(buckets.to_vec());
        Ok(())
    }

    async fn latest(&self) -> Result<Option<UsageEntity>> {
        Ok(None)
    }
}

fn fake(holder: &'static str) -> Fake {
    Fake {
        holder,
        measured: vec![BucketUsageEntity {
            bucket: "photos".to_owned(),
            objects: 3,
            bytes: 30,
        }],
        stored: Mutex::new(None),
    }
}

#[tokio::test]
async fn the_claim_holder_measures_and_stores() {
    let repository = fake("member-1");
    assert_eq!(
        refresh(&repository, "member-1").await.expect("refreshed"),
        Some(1)
    );
    let stored = repository.stored.lock().expect("unpoisoned").clone();
    assert_eq!(stored, Some(repository.measured.clone()));
}

#[tokio::test]
async fn a_member_without_the_claim_stores_nothing() {
    let repository = fake("member-1");
    assert_eq!(refresh(&repository, "member-2").await.expect("asked"), None);
    assert_eq!(*repository.stored.lock().expect("unpoisoned"), None);
}
