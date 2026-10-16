//! Object Lock: a version's retention (a mode and a retain-until instant), its legal hold, and a bucket's default
//! retention. The rules that keep a retained version are enforced by the metadata store itself (ADR-0016); these
//! types are what a request and a record say.

use tessari_s3_types::Timestamp;

/// How strictly a retention binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nobody shortens or removes it, and the version is kept until it passes.
    Compliance,
    /// Kept unless the caller may bypass governance retention and asks to.
    Governance,
}

impl Mode {
    /// The S3 spelling, also the stored one.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compliance => "COMPLIANCE",
            Self::Governance => "GOVERNANCE",
        }
    }

    /// The mode S3 spells `mode`; `None` for anything else.
    #[must_use]
    pub fn parse(mode: &str) -> Option<Self> {
        match mode {
            "COMPLIANCE" => Some(Self::Compliance),
            "GOVERNANCE" => Some(Self::Governance),
            _ => None,
        }
    }
}

/// A legal hold: on until somebody turns it off, whatever the retention says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegalHold {
    /// The version is kept.
    On,
    /// No hold.
    Off,
}

impl LegalHold {
    /// The S3 spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => "ON",
            Self::Off => "OFF",
        }
    }

    /// The hold S3 spells `status`; `None` for anything else.
    #[must_use]
    pub fn parse(status: &str) -> Option<Self> {
        match status {
            "ON" => Some(Self::On),
            "OFF" => Some(Self::Off),
            _ => None,
        }
    }
}

const MAX_DAYS: u16 = 36_500;
const MAX_YEARS: u16 = 100;
const DAYS_PER_YEAR: i64 = 365;
const SECONDS_PER_DAY: i64 = 86_400;

/// A default retention period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// Whole days, 1 to 36 500.
    Days(u16),
    /// Whole years of 365 days, 1 to 100.
    Years(u16),
}

/// A bucket's default retention, applied to a version written without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultRetention {
    /// The mode a default retention is written in.
    pub mode: Mode,
    /// How long from the write.
    pub period: Period,
}

impl DefaultRetention {
    /// A default within S3's limits; `None` outside them.
    #[must_use]
    pub const fn new(mode: Mode, period: Period) -> Option<Self> {
        let valid = match period {
            Period::Days(days) => days >= 1 && days <= MAX_DAYS,
            Period::Years(years) => years >= 1 && years <= MAX_YEARS,
        };
        if valid {
            Some(Self { mode, period })
        } else {
            None
        }
    }

    /// The period in seconds; `None` if it does not fit.
    #[must_use]
    pub fn seconds(&self) -> Option<u64> {
        let days = match self.period {
            Period::Days(days) => u64::from(days),
            Period::Years(years) => u64::from(years).checked_mul(DAYS_PER_YEAR.unsigned_abs())?,
        };
        days.checked_mul(SECONDS_PER_DAY.unsigned_abs())
    }

    /// The retain-until instant for a version written at `written`; `None` if it does not fit a timestamp.
    #[must_use]
    pub fn until(&self, written: Timestamp) -> Option<Timestamp> {
        let days = match self.period {
            Period::Days(days) => i64::from(days),
            Period::Years(years) => i64::from(years).checked_mul(DAYS_PER_YEAR)?,
        };
        let seconds = written
            .seconds
            .checked_add(days.checked_mul(SECONDS_PER_DAY)?)?;
        Some(Timestamp {
            seconds,
            nanos: written.nanos,
        })
    }
}

/// A bucket's Object Lock: once on, never off (and its versioning stays enabled), with an optional default retention.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Configuration {
    /// The retention a version written without one gets.
    pub default: Option<DefaultRetention>,
}

#[cfg(test)]
#[path = "object_lock_tests.rs"]
mod tests;
