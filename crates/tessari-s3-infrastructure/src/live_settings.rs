//! The settings a running node re-reads: each is held in a cell its readers consult on every use, so a value changed
//! in the metadata store reaches the daemons and the request paths without a restart.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;

use crate::S3Config;

/// The variables a running node applies without a restart; every other stored setting is read at start.
pub const APPLIED_LIVE: [&str; 8] = [
    "TESSARIDB_S3_RECLAIM_INTERVAL_SECS",
    "TESSARIDB_S3_UPLOAD_MAX_AGE_SECS",
    "TESSARIDB_S3_USAGE_INTERVAL_SECS",
    "TESSARIDB_S3_HEAL_INTERVAL_SECS",
    "TESSARIDB_S3_MOVE_BYTES_PER_SEC",
    "TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE",
    "TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE",
    "TESSARIDB_S3_DOMAINS",
];

/// One live number. Relaxed ordering: every cell stands alone and publishes nothing beside its own value.
#[derive(Debug, Clone)]
pub struct Live(Arc<AtomicU64>);

impl Live {
    /// A cell holding `value`.
    #[must_use]
    pub fn new(value: u64) -> Self {
        Self(Arc::new(AtomicU64::new(value)))
    }

    /// The value now.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    /// The value now, as seconds.
    #[must_use]
    pub fn secs(&self) -> Duration {
        Duration::from_secs(self.get())
    }

    /// Stores `value` for every holder of the cell, answering whether it differs from what the cell held.
    pub fn set(&self, value: u64) -> bool {
        self.0.swap(value, Ordering::Relaxed) != value
    }
}

/// The live settings of one node, published from each configuration it reads.
#[derive(Debug)]
pub struct LiveSettings {
    reclaim_interval: Live,
    upload_max_age: Live,
    usage_interval: Live,
    heal_interval: Live,
    move_bytes_per_sec: Live,
    console_sign_ins: Live,
    console_requests: Live,
    domains: ArcSwap<Vec<String>>,
}

impl LiveSettings {
    /// The live settings as `config` states them.
    #[must_use]
    pub fn new(config: &S3Config) -> Self {
        let values = Values::of(config);
        Self {
            reclaim_interval: Live::new(values.reclaim_interval),
            upload_max_age: Live::new(values.upload_max_age),
            usage_interval: Live::new(values.usage_interval),
            heal_interval: Live::new(values.heal_interval),
            move_bytes_per_sec: Live::new(values.move_bytes_per_sec),
            console_sign_ins: Live::new(values.console_sign_ins),
            console_requests: Live::new(values.console_requests),
            domains: ArcSwap::from_pointee(config.domains.clone()),
        }
    }

    /// Publishes `config`'s values, answering the variables whose value changed.
    pub fn publish(&self, config: &S3Config) -> Vec<&'static str> {
        let values = Values::of(config);
        let cells = [
            (
                &self.reclaim_interval,
                values.reclaim_interval,
                "TESSARIDB_S3_RECLAIM_INTERVAL_SECS",
            ),
            (
                &self.upload_max_age,
                values.upload_max_age,
                "TESSARIDB_S3_UPLOAD_MAX_AGE_SECS",
            ),
            (
                &self.usage_interval,
                values.usage_interval,
                "TESSARIDB_S3_USAGE_INTERVAL_SECS",
            ),
            (
                &self.heal_interval,
                values.heal_interval,
                "TESSARIDB_S3_HEAL_INTERVAL_SECS",
            ),
            (
                &self.move_bytes_per_sec,
                values.move_bytes_per_sec,
                "TESSARIDB_S3_MOVE_BYTES_PER_SEC",
            ),
            (
                &self.console_sign_ins,
                values.console_sign_ins,
                "TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE",
            ),
            (
                &self.console_requests,
                values.console_requests,
                "TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE",
            ),
        ];
        let mut changed: Vec<&'static str> = cells
            .into_iter()
            .filter(|(cell, value, _)| cell.set(*value))
            .map(|(_, _, key)| key)
            .collect();
        if **self.domains.load() != config.domains {
            self.domains.store(Arc::new(config.domains.clone()));
            changed.push("TESSARIDB_S3_DOMAINS");
        }
        changed
    }

    /// Seconds between reclaimer, upload-reaper and lifecycle runs.
    #[must_use]
    pub fn reclaim_interval(&self) -> Live {
        self.reclaim_interval.clone()
    }

    /// Seconds an unfinished upload is kept.
    #[must_use]
    pub fn upload_max_age(&self) -> Live {
        self.upload_max_age.clone()
    }

    /// Seconds between usage measurements.
    #[must_use]
    pub fn usage_interval(&self) -> Live {
        self.usage_interval.clone()
    }

    /// Seconds between heal and move runs on a cluster member.
    #[must_use]
    pub fn heal_interval(&self) -> Live {
        self.heal_interval.clone()
    }

    /// Bytes a second a layout move may send.
    #[must_use]
    pub fn move_bytes_per_sec(&self) -> Live {
        self.move_bytes_per_sec.clone()
    }

    /// Console sign-ins admitted per minute.
    #[must_use]
    pub fn console_sign_ins(&self) -> Live {
        self.console_sign_ins.clone()
    }

    /// Console requests admitted per minute.
    #[must_use]
    pub fn console_requests(&self) -> Live {
        self.console_requests.clone()
    }

    /// The virtual-host endpoint domains now.
    #[must_use]
    pub fn domains(&self) -> Arc<Vec<String>> {
        self.domains.load_full()
    }
}

/// The numbers one configuration states; a cluster's and a console's are zero where the node has neither, which no
/// reader consults.
struct Values {
    reclaim_interval: u64,
    upload_max_age: u64,
    usage_interval: u64,
    heal_interval: u64,
    move_bytes_per_sec: u64,
    console_sign_ins: u64,
    console_requests: u64,
}

impl Values {
    fn of(config: &S3Config) -> Self {
        let per_minute = |count: usize| u64::try_from(count).unwrap_or(u64::MAX);
        Self {
            reclaim_interval: config.reclaim_interval_secs,
            upload_max_age: config.upload_max_age_secs,
            usage_interval: config.usage_interval_secs,
            heal_interval: config
                .cluster
                .as_ref()
                .map_or(0, |cluster| cluster.heal_interval_secs),
            move_bytes_per_sec: config
                .cluster
                .as_ref()
                .map_or(0, |cluster| cluster.move_bytes_per_sec),
            console_sign_ins: config
                .console
                .as_ref()
                .map_or(0, |console| per_minute(console.sign_ins_per_minute)),
            console_requests: config
                .console
                .as_ref()
                .map_or(0, |console| per_minute(console.requests_per_minute)),
        }
    }
}

#[cfg(test)]
#[path = "live_settings_tests.rs"]
mod tests;
