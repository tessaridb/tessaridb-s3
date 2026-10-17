//! What the metrics are labelled and measured by: the five size classes, a served request, an authentication
//! refusal's reason, and the label sets — each drawn from a bounded set.

use prometheus_client::encoding::{EncodeLabelSet, LabelSetEncoder};

/// Size classes by their upper bound in bytes; a request at or over the last bound is `ge_128MiB`.
const SIZE_CLASSES: [(u64, &str); 4] = [
    (64 << 10, "lt_64KiB"),
    (1 << 20, "lt_1MiB"),
    (16 << 20, "lt_16MiB"),
    (128 << 20, "lt_128MiB"),
];

/// The size class of a request that moved `bytes` (the larger of its body and its response's).
#[must_use]
pub fn size_class(bytes: u64) -> &'static str {
    SIZE_CLASSES
        .iter()
        .find(|(bound, _)| bytes < *bound)
        .map_or("ge_128MiB", |(_, name)| name)
}

/// What one served request was.
#[derive(Debug, Clone, Copy)]
pub struct Served {
    /// Its operation's model name (`PutObject`), or `Unknown` before dispatch named one.
    pub operation: &'static str,
    /// The HTTP status answered.
    pub status: u16,
    /// The S3 error code, or `None` for a success.
    pub code: &'static str,
    /// The larger of the request body and the response body, in bytes.
    pub bytes: u64,
    /// Seconds from the request's arrival to its response.
    pub seconds: f64,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct RequestLabels {
    pub(crate) operation: &'static str,
    pub(crate) status: u16,
    pub(crate) code: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct DurationLabels {
    pub(crate) operation: &'static str,
    pub(crate) size: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct ReasonLabels {
    pub(crate) reason: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct OperationLabels {
    pub(crate) operation: &'static str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct SourceLabels {
    pub(crate) source: &'static str,
}

/// No labels: a gauge kept in a family so it has no sample until it is first set — a backlog never read shows as
/// absent, never as zero.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct Unlabelled;

impl EncodeLabelSet for Unlabelled {
    fn encode(&self, _encoder: &mut LabelSetEncoder<'_>) -> std::fmt::Result {
        Ok(())
    }
}

/// Why a request did not authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthRefusal {
    /// No signature at all.
    Anonymous,
    /// An access key no credential has, or a disabled one.
    UnknownKey,
    /// The request time is outside the allowed clock skew.
    Skew,
    /// A presigned URL past its expiry, or not valid yet.
    Expired,
    /// A signature that does not match the request.
    Signature,
    /// Any other malformed or unsupported authentication.
    Malformed,
}

impl AuthRefusal {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::UnknownKey => "unknown_key",
            Self::Skew => "skew",
            Self::Expired => "expired",
            Self::Signature => "signature",
            Self::Malformed => "malformed",
        }
    }
}

/// A gauge's value from a count, saturating at the largest a gauge holds.
pub(crate) fn gauge_value(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}
