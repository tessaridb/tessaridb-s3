//! What the pipeline learns about a request on its way through — the operation dispatch named, and, while the
//! access log is on, the bucket, the key and who signed — and the access-log record made from it once answered.

use std::net::SocketAddr;

use axum::extract::ConnectInfo;
use axum::http::Extensions;
use tessari_s3_storage::audit::AccessRecord;
use tokio::sync::mpsc::error::TrySendError;

use crate::state::ApiState;

/// A request's trail through the pipeline.
pub(crate) struct Trail {
    /// The operation dispatch named; `Unknown` until it did.
    pub(crate) operation: &'static str,
    /// Whether the access log is on, so the fields below are worth filling.
    pub(crate) logged: bool,
    /// The bucket addressed.
    pub(crate) bucket: Option<String>,
    /// The key addressed.
    pub(crate) key: Option<String>,
    /// The access key whose signature was verified.
    pub(crate) principal: Option<String>,
}

impl Trail {
    pub(crate) fn new(state: &ApiState) -> Self {
        Self {
            operation: "Unknown",
            logged: state.audit().is_some(),
            bucket: None,
            key: None,
            principal: None,
        }
    }
}

/// The client's address, when the connection recorded one.
pub(crate) fn client(extensions: &Extensions) -> Option<String> {
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip().to_string())
}

/// What the response said, for the access log.
pub(crate) struct Answered {
    pub(crate) request_id: String,
    pub(crate) at_secs: i64,
    pub(crate) method: String,
    pub(crate) client: Option<String>,
    pub(crate) status: u16,
    pub(crate) code: &'static str,
    pub(crate) bytes: u64,
    pub(crate) millis: u64,
}

/// Queues the request's access-log record, never waiting: a full queue counts the record as dropped.
pub(crate) fn log(state: &ApiState, trail: Trail, answered: Answered) {
    let Some(sink) = state.audit() else {
        return;
    };
    let record = AccessRecord {
        request_id: answered.request_id,
        at_secs: answered.at_secs,
        method: answered.method,
        operation: trail.operation,
        bucket: trail.bucket,
        key: trail.key,
        principal: trail
            .principal
            .unwrap_or_else(|| "unauthenticated".to_owned()),
        status: answered.status,
        code: answered.code,
        bytes: answered.bytes,
        millis: answered.millis,
        client: answered.client,
    };
    if let Err(TrySendError::Full(_) | TrySendError::Closed(_)) = sink.try_send(record) {
        state.metrics().audit_dropped();
    }
}
