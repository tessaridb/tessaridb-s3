//! The groups that read this crate's topics — on `object_events` one per notification destination and one for media
//! processing, on `replication_tasks` the replicators. A
//! group's name is grammar, never a bound value, so it is built only from a name this module checks first — narrower
//! than the store's own lexer — and positions are bound one parameter each.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};
use tessari_s3_types::Timestamp;

use crate::notifications::Event;
use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "object_events",
        reason,
    }
}

/// One handed-out message, exactly as the group answers it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MessageEntity {
    /// Its topic position: dense and commit-ordered.
    pub position: u64,
    /// How many times the group has handed it out, this time included.
    pub deliveries: u64,
    /// The message as the store's event wrote it.
    pub value: Value,
}

/// A group's quoted name, checked: `'<prefix><name>'` where the name is 1 to 64 letters, digits, `-` or `_` — and
/// the topic it reads, a name of this crate's schema.
#[derive(Debug, Clone)]
pub(crate) struct Group {
    quoted: String,
    topic: &'static str,
}

impl Group {
    /// A group on `object_events`.
    pub(crate) fn named(prefix: &str, name: &str) -> Result<Self> {
        Self::on("object_events", prefix, name)
    }

    /// A group on `topic`.
    pub(crate) fn on(topic: &'static str, prefix: &str, name: &str) -> Result<Self> {
        if !is_plain_name(name) {
            return Err(malformed(
                "a group name is 1 to 64 letters, digits, '-' or '_'",
            ));
        }
        Ok(Self {
            quoted: format!("'{prefix}{name}'"),
            topic,
        })
    }
}

/// Whether `name` is 1 to 64 letters, digits, `-` or `_` — the only names this crate quotes into a statement.
pub(crate) fn is_plain_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// How a group holds its messages: the deadline, the hand-outs before the dead letter, how many at once.
pub(crate) struct Holding {
    pub ack_deadline_secs: u64,
    pub deliveries: u32,
    pub in_flight: usize,
    /// The topic a message given up goes to; a name of this crate's schema, never a caller's.
    pub dead_letter: &'static str,
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

/// Declares `group` on its topic when it is missing; a new group starts at the topic's oldest message.
pub(crate) async fn define(pool: &MetaPool, group: &Group, holding: &Holding) -> Result<()> {
    let script = format!(
        "DEFINE GROUP IF NOT EXISTS {} ON TOPIC {} ACK DEADLINE {}s DELIVERIES {} IN FLIGHT {} \
         DEAD LETTER TO {};",
        group.quoted,
        group.topic,
        holding.ack_deadline_secs,
        holding.deliveries,
        holding.in_flight,
        holding.dead_letter
    );
    pool.run(&script, Vec::new()).await?;
    Ok(())
}

/// Hands out up to `limit` messages to `group`.
pub(crate) async fn read(
    pool: &MetaPool,
    group: &Group,
    limit: usize,
) -> Result<Vec<MessageEntity>> {
    let script = format!(
        "READ FROM {} FOR CONSUMER {} LIMIT $limit;",
        group.topic, group.quoted
    );
    let parameters = vec![("limit".to_owned(), crate::params::count(limit))];
    let answers = pool.run(&script, parameters).await?;
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

/// Settles `settled` for good.
pub(crate) async fn ack(pool: &MetaPool, group: &Group, settled: &[u64]) -> Result<()> {
    if settled.is_empty() {
        return Ok(());
    }
    let (at, parameters) = positions(settled)?;
    let script = format!("ACK {} FOR CONSUMER {} AT {at};", group.topic, group.quoted);
    pool.run(&script, parameters).await?;
    Ok(())
}

/// Hands `failed` out again no sooner than `delay_secs` from now.
pub(crate) async fn nack(
    pool: &MetaPool,
    group: &Group,
    failed: &[u64],
    delay_secs: u64,
) -> Result<()> {
    if failed.is_empty() {
        return Ok(());
    }
    // `DELAY` takes a literal duration; the number is the server's own, never a caller's.
    let (at, parameters) = positions(failed)?;
    let script = format!(
        "NACK {} FOR CONSUMER {} AT {at} DELAY {}s;",
        group.topic,
        group.quoted,
        delay_secs.max(1)
    );
    pool.run(&script, parameters).await?;
    Ok(())
}

/// The change a message records.
pub(crate) fn event(message: MessageEntity) -> Result<Event> {
    let Value::Object(fields) = message.value else {
        return Err(malformed("a message is not an object"));
    };
    let text = |field: &'static str| match fields.get(field) {
        Some(Value::String(text)) => Ok(Some(text.clone())),
        None | Some(Value::None) => Ok(None),
        Some(_) => Err(malformed(field)),
    };
    let required = |field: &'static str| text(field)?.ok_or_else(|| malformed(field));
    let size = match fields.get("size") {
        Some(Value::Number(Number::Integer(size))) => {
            Some(u64::try_from(*size).map_err(|_| malformed("size"))?)
        }
        None | Some(Value::None) => None,
        Some(_) => return Err(malformed("size")),
    };
    let at = match fields.get("at") {
        Some(Value::Datetime { seconds, nanos }) => Timestamp {
            seconds: *seconds,
            nanos: *nanos,
        },
        _ => return Err(malformed("at")),
    };
    Ok(Event {
        position: message.position,
        deliveries: message.deliveries,
        bucket: required("bucket_name")?,
        key: required("key")?,
        name: required("event")?,
        version_id: text("version_id")?,
        size,
        etag: text("etag")?,
        at,
    })
}

/// Seconds before a message handed out `deliveries` times is tried again: doubling from `step`, capped at `max`.
pub(crate) fn retry_after(deliveries: u64, step: u64, max: u64) -> u64 {
    let doublings = u32::try_from(deliveries.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(16);
    step.saturating_mul(1_u64 << doublings).min(max)
}
