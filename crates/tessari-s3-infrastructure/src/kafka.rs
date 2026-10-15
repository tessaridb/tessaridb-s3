//! Delivering a notification to an operator-declared Kafka topic. The connection is made on the first delivery and
//! shared by every delivery after it; a record goes to the partition its key hashes to, so the changes of one key keep
//! their order. Every delivery is bounded in time, a broker that does not answer counting as a failed delivery.

use std::hash::Hasher as _;
use std::time::Duration;

use rskafka::BackoffConfig;
use rskafka::client::ClientBuilder;
use rskafka::client::partition::{Compression, PartitionClient, UnknownTopicHandling};
use rskafka::record::Record;
use siphasher::sip::SipHasher13;
use tessari_s3_constants::NOTIFY_DELIVERY_TIMEOUT_SECS;
use tokio::sync::OnceCell;

/// A producer to one topic; share it behind an `Arc` — every clone of the connection is made once.
pub struct KafkaProducer {
    brokers: Vec<String>,
    topic: String,
    partitions: OnceCell<Vec<PartitionClient>>,
}

impl std::fmt::Debug for KafkaProducer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KafkaProducer")
            .field("brokers", &self.brokers)
            .field("topic", &self.topic)
            .finish_non_exhaustive()
    }
}

/// Why a connection to the topic could not be made.
#[derive(Debug, thiserror::Error)]
enum Unusable {
    #[error("{0}")]
    Client(#[from] rskafka::client::error::Error),
    #[error("the topic does not exist on the brokers")]
    NoTopic,
}

impl KafkaProducer {
    /// A producer to `topic` on `brokers`; nothing is dialled until the first delivery.
    #[must_use]
    pub fn new(brokers: Vec<String>, topic: String) -> Self {
        Self {
            brokers,
            topic,
            partitions: OnceCell::new(),
        }
    }

    async fn connect(&self) -> Result<Vec<PartitionClient>, Unusable> {
        let timeout = Duration::from_secs(NOTIFY_DELIVERY_TIMEOUT_SECS);
        let client = ClientBuilder::new(self.brokers.clone())
            .backoff_config(BackoffConfig {
                deadline: Some(timeout),
                ..BackoffConfig::default()
            })
            .build()
            .await?;
        let topic = client
            .list_topics()
            .await?
            .into_iter()
            .find(|topic| topic.name == self.topic)
            .ok_or(Unusable::NoTopic)?;
        let mut partitions = Vec::with_capacity(topic.partitions.len());
        for partition in topic.partitions {
            partitions.push(
                client
                    .partition_client(self.topic.clone(), partition, UnknownTopicHandling::Error)
                    .await?,
            );
        }
        if partitions.is_empty() {
            return Err(Unusable::NoTopic);
        }
        Ok(partitions)
    }

    /// Produces `value` under `key`, stamped with the change's time `at` (seconds and nanoseconds since the epoch);
    /// `true` only once the broker acknowledged it.
    pub async fn send(&self, key: &str, value: bytes::Bytes, at: (i64, u32)) -> bool {
        let deadline = Duration::from_secs(NOTIFY_DELIVERY_TIMEOUT_SECS);
        let produce = async {
            let partitions = self.partitions.get_or_try_init(|| self.connect()).await?;
            let mut hasher = SipHasher13::new();
            hasher.write(key.as_bytes());
            let count = u64::try_from(partitions.len()).unwrap_or(1).max(1);
            let index =
                usize::try_from(hasher.finish().checked_rem(count).unwrap_or(0)).unwrap_or(0);
            let record = Record {
                key: Some(key.as_bytes().to_vec()),
                value: Some(value.to_vec()),
                headers: std::collections::BTreeMap::new(),
                timestamp: rskafka::chrono::DateTime::from_timestamp(at.0, at.1)
                    .unwrap_or_default(),
            };
            partitions
                .get(index)
                .ok_or(Unusable::NoTopic)?
                .produce(vec![record], Compression::NoCompression)
                .await?;
            Ok::<(), Unusable>(())
        };
        match tokio::time::timeout(deadline, produce).await {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(topic = self.topic.as_str(), %error, "a Kafka destination refused a notification");
                false
            }
            Err(_) => {
                tracing::warn!(
                    topic = self.topic.as_str(),
                    "a Kafka destination did not answer in time"
                );
                false
            }
        }
    }
}
