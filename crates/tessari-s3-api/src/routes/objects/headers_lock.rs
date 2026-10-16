//! What Object Lock a write asks for in its headers (ADR-0016): a retention and a legal hold, or nothing.

use axum::http::HeaderMap;
use tessari_s3_core::object_lock::{LegalHold, Mode};
use tessari_s3_storage::objects::{Lock, RetainUntil};
use tessari_s3_types::{ErrorCode, Timestamp};

use super::not_implemented;
use crate::{Error, Result};

/// The Object Lock a write asks for in its headers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LockAsked {
    /// A retention: both `x-amz-object-lock-mode` and `x-amz-object-lock-retain-until-date`.
    pub(crate) retention: Option<(Mode, Timestamp)>,
    /// `x-amz-object-lock-legal-hold: ON`.
    pub(crate) legal_hold: bool,
    /// Whether any Object Lock header was sent — which a bucket without Object Lock refuses.
    pub(crate) asked: bool,
}

impl LockAsked {
    /// What the version is written with.
    pub(crate) fn lock(self) -> Lock {
        Lock {
            retention: self
                .retention
                .map(|(mode, until)| (mode, RetainUntil::At(until))),
            legal_hold: self.legal_hold,
        }
    }
}

/// Reads the Object Lock headers: a mode and a future retain-until together or neither, a legal hold `ON` or `OFF`;
/// an event hold is refused as not implemented.
pub(super) fn read_lock(headers: &HeaderMap) -> Result<LockAsked> {
    let text = |name: &str| headers.get(name).map(|value| value.to_str().map(str::trim));
    let invalid = |message: &str| Error::new(ErrorCode::InvalidArgument, message.to_owned());
    if headers
        .keys()
        .any(|name| name.as_str().starts_with("x-amz-object-lock-event-hold"))
    {
        return Err(not_implemented("an Object Lock event hold"));
    }
    let mode = text("x-amz-object-lock-mode")
        .transpose()
        .map_err(|_| invalid("a lock header is not ASCII"))?;
    let until = text("x-amz-object-lock-retain-until-date")
        .transpose()
        .map_err(|_| invalid("a lock header is not ASCII"))?;
    let hold = text("x-amz-object-lock-legal-hold")
        .transpose()
        .map_err(|_| invalid("a lock header is not ASCII"))?;
    let retention = match (mode, until) {
        (None, None) => None,
        (Some(mode), Some(until)) => {
            let mode = Mode::parse(mode)
                .ok_or_else(|| invalid("x-amz-object-lock-mode is GOVERNANCE or COMPLIANCE"))?;
            let until = Timestamp::parse_iso8601(until).ok_or_else(|| {
                invalid("x-amz-object-lock-retain-until-date is an ISO 8601 instant")
            })?;
            if until <= Timestamp::now() {
                return Err(invalid("The retain until date must be in the future"));
            }
            Some((mode, until))
        }
        _ => {
            return Err(invalid(
                "x-amz-object-lock-mode and x-amz-object-lock-retain-until-date go together",
            ));
        }
    };
    let legal_hold = match hold.map(LegalHold::parse) {
        None => false,
        Some(Some(hold)) => hold == LegalHold::On,
        Some(None) => return Err(invalid("x-amz-object-lock-legal-hold is ON or OFF")),
    };
    Ok(LockAsked {
        retention,
        legal_hold,
        asked: retention.is_some() || hold.is_some(),
    })
}
