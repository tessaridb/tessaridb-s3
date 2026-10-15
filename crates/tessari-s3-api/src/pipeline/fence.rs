//! The region fence: a region an operator failed over to another refuses client writes until it is restored, so two
//! regions never take writes for one bucket. Reads go on, and a replica — the promoted region catching this one up —
//! is still taken. A store that cannot say whether the region is failed over refuses the write (its own error).

use tessari_s3_core::dispatch::Method;
use tessari_s3_types::ErrorCode;

use crate::state::ApiState;
use crate::{Error, Result};

/// `Ok` when a request of `method` may run in this node's region; `replica` says it carries a replica's version id.
///
/// # Errors
/// `ServiceUnavailable` naming the promoted region and the epoch while the region is failed over; the metadata store's
/// error when it cannot answer.
pub(super) async fn writable(state: &ApiState, method: Method, replica: bool) -> Result<()> {
    if matches!(method, Method::Get | Method::Head) || replica {
        return Ok(());
    }
    match state
        .storage()
        .regions()
        .failed_over(state.region())
        .await?
    {
        Some(region) => Err(Error::new(
            ErrorCode::ServiceUnavailable,
            format!(
                "region {} was failed over to {} at epoch {}; write there",
                region.region,
                region.promoted.as_deref().unwrap_or_default(),
                region.epoch
            ),
        )),
        None => Ok(()),
    }
}
