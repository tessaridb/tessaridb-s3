//! Each destination's group on the `object_events` topic in TessariDB, named after the destination's id, and the
//! default destination's own topic `s3_notifications`.

use tessari_s3_constants::{NOTIFY_ACK_DEADLINE_SECS, NOTIFY_DELIVERIES, NOTIFY_IN_FLIGHT};
use tessari_s3_infrastructure::tessaridb::{MetaPool, RefusalClass, Value};

use super::repository::NotificationRepository;
use crate::Result;
use crate::topic_groups::{self, Group, Holding, MessageEntity};

/// The destinations' groups over a metadata pool.
#[derive(Clone)]
pub struct TessariNotifications {
    pool: MetaPool,
}

impl TessariNotifications {
    /// The groups over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

/// The group `target` reads under: `'notify-<target>'`.
fn group(target: &str) -> Result<Group> {
    Group::named("notify-", target)
}

const HOLDING: Holding = Holding {
    ack_deadline_secs: NOTIFY_ACK_DEADLINE_SECS,
    deliveries: NOTIFY_DELIVERIES,
    in_flight: NOTIFY_IN_FLIGHT,
    dead_letter: "object_events_dead",
};

impl NotificationRepository for TessariNotifications {
    async fn define(&self, target: &str) -> Result<()> {
        topic_groups::define(&self.pool, &group(target)?, &HOLDING).await
    }

    async fn read(&self, target: &str, limit: usize) -> Result<Vec<MessageEntity>> {
        topic_groups::read(&self.pool, &group(target)?, limit).await
    }

    async fn publish(&self, position: u64, body: &str) -> Result<()> {
        let parameters = vec![
            ("name".to_owned(), Value::String(format!("n-{position}"))),
            ("body".to_owned(), Value::String(body.to_owned())),
        ];
        match self
            .pool
            .run(
                "CREATE s3_notifications:$name = json::parse($body);",
                parameters,
            )
            .await
        {
            Ok(_) => Ok(()),
            // Named by the event: a create that collides is the earlier delivery of this same event.
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn ack(&self, target: &str, settled: &[u64]) -> Result<()> {
        topic_groups::ack(&self.pool, &group(target)?, settled).await
    }

    async fn nack(&self, target: &str, failed: &[u64], delay_secs: u64) -> Result<()> {
        topic_groups::nack(&self.pool, &group(target)?, failed, delay_secs).await
    }
}
