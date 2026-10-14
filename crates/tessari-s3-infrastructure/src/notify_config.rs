//! The notification half of the configuration: the destinations an operator declares for bucket event notifications.
//! A bucket's configuration names a destination by its id and never by an address, so a bucket administrator cannot
//! point the server at a host of their choosing.

use crate::{Error, Result};

const KEY: &str = "TESSARIDB_S3_NOTIFY_TARGETS";

/// Where one declared destination delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyEndpoint {
    /// An HTTP endpoint each event is posted to.
    Webhook(String),
    /// A Kafka topic on these brokers.
    Kafka {
        /// `host:port` of each broker.
        brokers: Vec<String>,
        /// The topic events are produced to.
        topic: String,
    },
}

/// One destination an operator declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyTarget {
    /// The id a bucket's configuration names it by.
    pub id: String,
    /// Where it delivers.
    pub endpoint: NotifyEndpoint,
}

/// `TESSARIDB_S3_NOTIFY_TARGETS`: unset, or targets separated by `;`.
///
/// # Errors
/// [`crate::Error::InvalidConfig`] naming what is wrong; the value is never echoed.
pub(crate) fn notify_targets(get: &impl Fn(&str) -> Option<String>) -> Result<Vec<NotifyTarget>> {
    let Some(text) = get(KEY) else {
        return Ok(Vec::new());
    };
    let mut targets: Vec<NotifyTarget> = Vec::new();
    for entry in text
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let target = target(entry)?;
        if targets.iter().any(|seen| seen.id == target.id) {
            return Err(invalid("two targets share an id"));
        }
        targets.push(target);
    }
    Ok(targets)
}

const fn invalid(reason: &'static str) -> Error {
    Error::InvalidConfig { key: KEY, reason }
}

fn target(entry: &str) -> Result<NotifyTarget> {
    let shape = || invalid("a target is <id>=<webhook|kafka>:<endpoint>");
    let (id, rest) = entry.split_once('=').ok_or_else(shape)?;
    let (kind, endpoint) = rest.split_once(':').ok_or_else(shape)?;
    let id = id.trim();
    let named = (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !named {
        return Err(invalid(
            "a target id is 1 to 64 letters, digits, '-' or '_'",
        ));
    }
    // The same name as the core crate's `notification::DATABASE_TARGET`, which this crate does not depend on.
    if id == "tessaridb" {
        return Err(invalid(
            "'tessaridb' names the database's own topic and cannot be declared",
        ));
    }
    let endpoint = match kind.trim() {
        "webhook" => webhook(endpoint.trim())?,
        "kafka" => kafka(endpoint.trim())?,
        _ => return Err(invalid("a target kind is webhook or kafka")),
    };
    Ok(NotifyTarget {
        id: id.to_owned(),
        endpoint,
    })
}

fn webhook(url: &str) -> Result<NotifyEndpoint> {
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or_default());
    match host {
        Some(host) if !host.is_empty() => Ok(NotifyEndpoint::Webhook(url.to_owned())),
        _ => Err(invalid(
            "a webhook is an http:// or https:// URL with a host",
        )),
    }
}

fn kafka(endpoint: &str) -> Result<NotifyEndpoint> {
    let shape = || invalid("a kafka target is <host:port>[,<host:port>…]/<topic>");
    let (brokers, topic) = endpoint.rsplit_once('/').ok_or_else(shape)?;
    let brokers: Vec<String> = brokers
        .split(',')
        .map(|broker| broker.trim().to_owned())
        .collect();
    let addressed = |broker: &String| {
        broker
            .rsplit_once(':')
            .is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok())
    };
    if topic.trim().is_empty() || !brokers.iter().all(addressed) {
        return Err(shape());
    }
    Ok(NotifyEndpoint::Kafka {
        brokers,
        topic: topic.trim().to_owned(),
    })
}

#[cfg(test)]
#[path = "notify_config_tests.rs"]
mod tests;
