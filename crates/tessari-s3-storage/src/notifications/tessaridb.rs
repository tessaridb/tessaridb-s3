//! Each destination's group on the `object_events` topic in TessariDB. A group's name is grammar, never a bound value,
//! so it is built only from a target id this module checks first — narrower than the store's own lexer.

use tessari_s3_constants::{NOTIFY_ACK_DEADLINE_SECS, NOTIFY_DELIVERIES, NOTIFY_IN_FLIGHT};
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};

use super::entity::MessageEntity;
use super::repository::NotificationRepository;
use crate::{Error, Result};

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

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "object_events",
        reason,
    }
}

/// The group `target` reads under: `'notify-<target>'`, the quotes included.
fn group(target: &str) -> Result<String> {
    let named = (1..=64).contains(&target.len())
        && target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !named {
        return Err(malformed(
            "a target id is 1 to 64 letters, digits, '-' or '_'",
        ));
    }
    Ok(format!("'notify-{target}'"))
}

/// `$p0, $p1, …` and their bound values: `AT` takes a list of positions and refuses an array, so each is its own
/// parameter (the names are the server's, the values bound).
fn positions(positions: &[u64]) -> Result<(String, Vec<(String, Value)>)> {
    let mut names = Vec::with_capacity(positions.len());
    let mut parameters = Vec::with_capacity(positions.len());
    for (index, position) in positions.iter().enumerate() {
        let position = i64::try_from(*position).map_err(|_| malformed("a position past i64"))?;
        names.push(format!("$p{index}"));
        parameters.push((
            format!("p{index}"),
            Value::Number(Number::Integer(position)),
        ));
    }
    Ok((names.join(", "), parameters))
}

fn whole(value: Option<&Value>, reason: &'static str) -> Result<u64> {
    match value {
        Some(Value::Number(Number::Integer(n))) => u64::try_from(*n).map_err(|_| malformed(reason)),
        _ => Err(malformed(reason)),
    }
}

impl NotificationRepository for TessariNotifications {
    async fn define(&self, target: &str) -> Result<()> {
        let script = format!(
            "DEFINE GROUP IF NOT EXISTS {} ON TOPIC object_events ACK DEADLINE {NOTIFY_ACK_DEADLINE_SECS}s \
             DELIVERIES {NOTIFY_DELIVERIES} IN FLIGHT {NOTIFY_IN_FLIGHT} DEAD LETTER TO object_events_dead;",
            group(target)?
        );
        self.pool.run(&script, Vec::new()).await?;
        Ok(())
    }

    async fn read(&self, target: &str, limit: usize) -> Result<Vec<MessageEntity>> {
        let script = format!(
            "READ FROM object_events FOR CONSUMER {} LIMIT $limit;",
            group(target)?
        );
        let parameters = vec![("limit".to_owned(), crate::params::count(limit))];
        let answers = self.pool.run(&script, parameters).await?;
        let records = match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => records,
            _ => return Ok(Vec::new()),
        };
        records
            .into_iter()
            .map(|(_, record)| {
                let Value::Object(mut fields) = record else {
                    return Err(malformed("a handed-out message is not an object"));
                };
                Ok(MessageEntity {
                    position: whole(fields.get("position"), "position")?,
                    deliveries: whole(fields.get("deliveries"), "deliveries")?,
                    value: fields.remove("value").ok_or_else(|| malformed("value"))?,
                })
            })
            .collect()
    }

    async fn ack(&self, target: &str, settled: &[u64]) -> Result<()> {
        if settled.is_empty() {
            return Ok(());
        }
        let (at, parameters) = positions(settled)?;
        let script = format!("ACK object_events FOR CONSUMER {} AT {at};", group(target)?);
        self.pool.run(&script, parameters).await?;
        Ok(())
    }

    async fn nack(&self, target: &str, failed: &[u64], delay_secs: u64) -> Result<()> {
        if failed.is_empty() {
            return Ok(());
        }
        // `DELAY` takes a literal duration; the number is the server's own, never a caller's.
        let (at, parameters) = positions(failed)?;
        let script = format!(
            "NACK object_events FOR CONSUMER {} AT {at} DELAY {}s;",
            group(target)?,
            delay_secs.max(1)
        );
        self.pool.run(&script, parameters).await?;
        Ok(())
    }
}
