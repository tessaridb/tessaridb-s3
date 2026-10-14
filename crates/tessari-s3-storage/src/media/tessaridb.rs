//! The `media` group on the `object_events` topic in TessariDB.

use tessari_s3_constants::{MEDIA_ACK_DEADLINE_SECS, MEDIA_DELIVERIES, MEDIA_IN_FLIGHT};
use tessari_s3_infrastructure::tessaridb::MetaPool;

use super::repository::MediaRepository;
use crate::Result;
use crate::topic_groups::{self, Group, Holding, MessageEntity};

/// The media group over a metadata pool.
#[derive(Clone)]
pub struct TessariMedia {
    pool: MetaPool,
}

impl TessariMedia {
    /// The group over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn group() -> Result<Group> {
    Group::named("", "media")
}

const HOLDING: Holding = Holding {
    ack_deadline_secs: MEDIA_ACK_DEADLINE_SECS,
    deliveries: MEDIA_DELIVERIES,
    in_flight: MEDIA_IN_FLIGHT,
    dead_letter: "media_failed",
};

impl MediaRepository for TessariMedia {
    async fn define(&self) -> Result<()> {
        topic_groups::define(&self.pool, &group()?, &HOLDING).await
    }

    async fn read(&self, limit: usize) -> Result<Vec<MessageEntity>> {
        topic_groups::read(&self.pool, &group()?, limit).await
    }

    async fn ack(&self, settled: &[u64]) -> Result<()> {
        topic_groups::ack(&self.pool, &group()?, settled).await
    }

    async fn nack(&self, failed: &[u64], delay_secs: u64) -> Result<()> {
        topic_groups::nack(&self.pool, &group()?, failed, delay_secs).await
    }
}
