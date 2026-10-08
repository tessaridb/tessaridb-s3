//! Signature verification for every internal request, before routing reaches a handler: the calling node, the date
//! and the signature headers must all be present and the signature must be the cluster's over this method, path
//! and query, within the skew window. Anything else is 401 and nothing is touched.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tessari_s3_constants::{INTERNAL_DATE_HEADER, INTERNAL_NODE_HEADER, INTERNAL_SIGNATURE_HEADER};
use tessari_s3_core::internal::{InternalRequest, verify};

use super::InternalState;
use super::refusal::Refusal;

/// Runs the request only when it is signed by the cluster.
pub(super) async fn authenticate(
    State(state): State<InternalState>,
    request: Request,
    next: Next,
) -> Response {
    if signed_by_cluster(&state, &request) {
        next.run(request).await
    } else {
        Refusal::Unauthorized.into_response()
    }
}

/// Whether `request` carries the three headers and a signature the cluster secret verifies.
fn signed_by_cluster(state: &InternalState, request: &Request) -> bool {
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let (Some(node), Some(date), Some(signature)) = (
        header(INTERNAL_NODE_HEADER),
        header(INTERNAL_DATE_HEADER).and_then(|date| date.parse::<i64>().ok()),
        header(INTERNAL_SIGNATURE_HEADER),
    ) else {
        tracing::warn!("internal request without a node, a date or a signature");
        return false;
    };
    let signed = InternalRequest {
        method: request.method().as_str(),
        path: request
            .uri()
            .path_and_query()
            .map_or("", |target| target.as_str()),
        date,
        node,
    };
    verify(state.secret(), &signed, signature, state.now())
        .inspect_err(|error| tracing::warn!(node, error = %error, "internal request refused"))
        .is_ok()
}
