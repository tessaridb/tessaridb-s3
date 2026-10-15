//! The replicators' group on this region's `replication_tasks` topic, the status stamps on its version records, and
//! the `s3_regions` registry in the global database.

use tessari_s3_constants::{
    REPLICATION_ACK_DEADLINE_SECS, REPLICATION_DELIVERIES, REPLICATION_IN_FLIGHT,
};
use tessari_s3_infrastructure::tessaridb::{MetaPool, RefusalClass, Value};

use super::repository::ReplicationRepository;
use crate::answers::first_record;
use crate::objects::version_id::{key_prefix, record_id};
use crate::topic_groups::{self, Group, Holding, MessageEntity};
use crate::{Error, Result};

/// Replication over the global database (`pool`) and this region's (`regional`).
#[derive(Clone)]
pub struct TessariReplication {
    pool: MetaPool,
    regional: MetaPool,
}

impl TessariReplication {
    /// Replication over the global `pool` and this region's `regional` handle.
    #[must_use]
    pub const fn new(pool: MetaPool, regional: MetaPool) -> Self {
        Self { pool, regional }
    }
}

fn group() -> Result<Group> {
    Group::on("replication_tasks", "", "replicators")
}

const HOLDING: Holding = Holding {
    ack_deadline_secs: REPLICATION_ACK_DEADLINE_SECS,
    deliveries: REPLICATION_DELIVERIES,
    in_flight: REPLICATION_IN_FLIGHT,
    dead_letter: "replication_failed",
};

/// Runs `script`, leaving alone a record that is gone or no longer what the condition names.
async fn stamp_if_there(
    pool: &MetaPool,
    script: &str,
    parameters: Vec<(String, Value)>,
) -> Result<()> {
    match pool.run(script, parameters).await {
        Ok(_) => Ok(()),
        Err(error) if error.is_class(RefusalClass::Conflict) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

impl ReplicationRepository for TessariReplication {
    async fn define(&self) -> Result<()> {
        topic_groups::define(&self.regional, &group()?, &HOLDING).await
    }

    async fn read(&self, limit: usize) -> Result<Vec<MessageEntity>> {
        topic_groups::read(&self.regional, &group()?, limit).await
    }

    async fn ack(&self, positions: &[u64]) -> Result<()> {
        topic_groups::ack(&self.regional, &group()?, positions).await
    }

    async fn nack(&self, positions: &[u64], delay_secs: u64) -> Result<()> {
        topic_groups::nack(&self.regional, &group()?, positions, delay_secs).await
    }

    async fn stamp(
        &self,
        bucket: &str,
        key: &str,
        seq: u64,
        version_id: &str,
        status: &'static str,
    ) -> Result<()> {
        let parameters = || {
            vec![
                ("status".to_owned(), Value::String(status.to_owned())),
                ("vid".to_owned(), Value::String(version_id.to_owned())),
            ]
        };
        let mut version = parameters();
        version.push((
            "at".to_owned(),
            Value::Bytes(record_id(&key_prefix(bucket, key), seq)),
        ));
        stamp_if_there(
            &self.regional,
            "UPDATE versions:$at SET replication_status = $status WHERE version_id = $vid;",
            version,
        )
        .await?;
        let mut object = parameters();
        object.push(("id".to_owned(), Value::String(format!("{bucket}/{key}"))));
        stamp_if_there(
            &self.regional,
            "UPDATE objects:$id SET replication_status = $status WHERE version_id = $vid;",
            object,
        )
        .await
    }

    async fn advertise(&self, region: &str, endpoint: &str) -> Result<()> {
        let parameters = vec![
            ("region".to_owned(), Value::String(region.to_owned())),
            ("endpoint".to_owned(), Value::String(endpoint.to_owned())),
        ];
        self.pool
            .run(
                "UPSERT s3_regions:$region SET endpoint = $endpoint, seen = time::now();",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn endpoint(&self, region: &str) -> Result<Option<String>> {
        let answers = self
            .pool
            .run(
                "SELECT endpoint FROM ONLY s3_regions:$region;",
                vec![("region".to_owned(), Value::String(region.to_owned()))],
            )
            .await?;
        match first_record(answers.into_iter().next())? {
            None => Ok(None),
            Some(Value::Object(fields)) => match fields.get("endpoint") {
                Some(Value::String(endpoint)) => Ok(Some(endpoint.clone())),
                _ => Err(Error::Malformed {
                    record: "s3_regions",
                    reason: "endpoint",
                }),
            },
            Some(_) => Err(Error::Malformed {
                record: "s3_regions",
                reason: "not an object",
            }),
        }
    }
}
