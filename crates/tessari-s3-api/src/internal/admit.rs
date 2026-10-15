//! Admission for every internal request, before routing reaches a handler: the certificate the connection presented
//! must speak for the node the request names (401 otherwise), and that node must be an admitted member presenting
//! that certificate (403 otherwise; 503 when membership cannot be read). An answer from the membership store is kept
//! for [`INTERNAL_ADMISSION_CACHE_SECS`] per node and certificate, which is how long a removal takes to reach a node.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures_util::future::BoxFuture;
use rustls::pki_types::CertificateDer;
use tessari_s3_constants::{INTERNAL_ADMISSION_CACHE_SECS, INTERNAL_NODE_HEADER};
use tessari_s3_infrastructure::credential;
use tessari_s3_storage::members::{Admission, Fingerprint, MembershipService};
use tessari_s3_types::NodeId;

use super::InternalState;
use super::refusal::Refusal;

/// The certificate the connection presented, if any, inserted into each request by the server.
#[derive(Debug, Clone)]
pub struct PeerCertificate(pub Option<CertificateDer<'static>>);

/// Whether a node presenting a certificate with a fingerprint is an admitted member of the cluster.
pub trait Admit: Send + Sync {
    /// The membership's answer for `node` presenting `fingerprint`.
    fn admits<'a>(
        &'a self,
        node: &'a NodeId,
        fingerprint: &'a Fingerprint,
    ) -> BoxFuture<'a, tessari_s3_storage::Result<Admission>>;
}

impl Admit for MembershipService {
    fn admits<'a>(
        &'a self,
        node: &'a NodeId,
        fingerprint: &'a Fingerprint,
    ) -> BoxFuture<'a, tessari_s3_storage::Result<Admission>> {
        Box::pin(MembershipService::admits(self, node, fingerprint))
    }
}

/// Runs the request only for an admitted member presenting its own certificate.
pub(super) async fn admit(
    State(state): State<InternalState>,
    request: Request,
    next: Next,
) -> Response {
    let admitted = match identify(&request) {
        Ok((node, fingerprint)) => admitted(&state, &node, &fingerprint).await,
        Err(refusal) => Err(refusal),
    };
    match admitted {
        Ok(()) => next.run(request).await,
        Err(refusal) => refusal.into_response(),
    }
}

/// The node `request` names and the fingerprint of the certificate its connection presented, once that certificate
/// speaks for that node.
fn identify(request: &Request) -> Result<(NodeId, Fingerprint), Refusal> {
    let certificate = request
        .extensions()
        .get::<PeerCertificate>()
        .and_then(|presented| presented.0.as_ref());
    let node = request
        .headers()
        .get(INTERNAL_NODE_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|node| NodeId::new(node).ok())
        .ok_or_else(|| {
            tracing::warn!("internal request without a valid node id");
            Refusal::Unidentified
        })?;
    if let Err(refused) = credential::speaks_for(certificate, &node) {
        tracing::warn!(node = %node.as_str(), error = %refused, "internal request refused");
        return Err(Refusal::Unidentified);
    }
    // `speaks_for` succeeded, so a certificate was presented.
    let certificate = certificate.ok_or(Refusal::Unidentified)?;
    let fingerprint = Fingerprint::new(&credential::fingerprint(certificate)).ok_or_else(|| {
        tracing::error!("a certificate fingerprint was not in the fingerprint form");
        Refusal::Internal
    })?;
    Ok((node, fingerprint))
}

/// Whether `node` presenting `fingerprint` is an admitted member.
async fn admitted(
    state: &InternalState,
    node: &NodeId,
    fingerprint: &Fingerprint,
) -> Result<(), Refusal> {
    match answer(state, node, fingerprint).await {
        Ok(Admission::Admitted) => Ok(()),
        Ok(other) => {
            tracing::warn!(node = %node.as_str(), admission = ?other, "internal request from a node not admitted");
            Err(Refusal::NotAdmitted)
        }
        Err(error) => {
            tracing::error!(node = %node.as_str(), error = %error, "cluster membership could not be read");
            Err(Refusal::MembershipUnavailable)
        }
    }
}

/// The membership's answer for `node` and `fingerprint`: the kept one while younger than the cache window, otherwise
/// a fresh one, which replaces it. A failure to read is never kept. Only a certificate the cluster authority issued
/// for the node it names gets this far, so the entries grow with the certificates issued, not with the requests.
async fn answer(
    state: &InternalState,
    node: &NodeId,
    fingerprint: &Fingerprint,
) -> tessari_s3_storage::Result<Admission> {
    let now = state.now();
    let key = (node.as_str().to_owned(), fingerprint.as_str().to_owned());
    let kept = state
        .admissions()
        .read_async(&key, |_, &(admission, at)| (admission, at))
        .await;
    if let Some((admission, at)) = kept
        && now.saturating_sub(at) < INTERNAL_ADMISSION_CACHE_SECS
    {
        return Ok(admission);
    }
    let admission = state.admit().admits(node, fingerprint).await?;
    state.admissions().upsert_async(key, (admission, now)).await;
    Ok(admission)
}
