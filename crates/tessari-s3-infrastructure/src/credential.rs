//! What a peer's certificate is asked — never what it is read for.
//!
//! A node claims, on the internal surface, to be a node of the cluster; the certificate it presented over mutual TLS
//! is then asked one question: *is it valid for the name that node's peer credential must carry*. That is the question
//! every TLS client asks of a host, so the code answering it is webpki's and is audited, and no certificate field is
//! parsed here.
//!
//! The name is `<node>.peer.tessari-s3`. The link is in the name so a credential issued for another link never answers
//! for this one, and the realm keeps the name out of every resolver's namespace; it is never looked up. A node id is
//! a DNS label by construction (lowercase letters, digits, inner hyphens, at most 63 bytes).
//!
//! Asking says whether the certificate names the claimed node, never which node it names instead, so a refusal carries
//! the certificate's fingerprint — the one thing that finds the mis-issued file on the machine holding it.

use rustls::pki_types::{CertificateDer, ServerName};
use sha2::{Digest, Sha256};
use tessari_s3_types::{ErrorCategory, NodeId};

/// The private namespace every peer credential ends in.
const REALM: &str = "peer.tessari-s3";

/// Why a peer's certificate does not speak for the node it claims to be.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    /// No certificate was presented.
    #[error("the peer presented no certificate")]
    Unidentified,
    /// A certificate was presented and does not name the claimed node's peer credential.
    #[error("the certificate {fingerprint} does not speak for node {claimed}")]
    NamesAnother {
        /// The node the peer claimed to be.
        claimed: String,
        /// The presented certificate's SHA-256, lowercase hex.
        fingerprint: String,
    },
}

impl Refused {
    /// The taxonomy category: a refusal, never a fault.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        ErrorCategory::Forbidden
    }
}

/// The name `node`'s peer credential carries.
#[must_use]
pub fn names(node: &NodeId) -> String {
    format!("{}.{REALM}", node.as_str())
}

/// Whether `certificate` speaks for `claimed` on the peer link.
///
/// # Errors
/// [`Refused::Unidentified`] when nothing was presented; [`Refused::NamesAnother`] when the certificate is not valid
/// for `claimed`'s peer name — including bytes that are not a certificate at all.
pub fn speaks_for(
    certificate: Option<&CertificateDer<'_>>,
    claimed: &NodeId,
) -> Result<(), Refused> {
    let certificate = certificate.ok_or(Refused::Unidentified)?;
    if valid_for(certificate, &names(claimed)) {
        return Ok(());
    }
    Err(Refused::NamesAnother {
        claimed: claimed.as_str().to_owned(),
        fingerprint: fingerprint(certificate),
    })
}

/// Refuses a node whose own certificate does not speak for it, so a mis-issued file stops the node at start rather
/// than at every peer it calls.
///
/// # Errors
/// [`crate::Error::InvalidConfig`] naming the certificate file when it holds no certificate or one issued for
/// another node.
pub fn speaks_for_itself(tls: &crate::tls::InternalTls, node: &NodeId) -> crate::Result<()> {
    let own = crate::tls::leaf(tls)?;
    speaks_for(Some(&own), node).map_err(|_| crate::Error::InvalidConfig {
        key: "TESSARIDB_S3_INTERNAL_TLS_CERT",
        reason: "issued for another node than TESSARIDB_S3_NODE_ID",
    })
}

/// The certificate's SHA-256 over its DER, lowercase hex.
#[must_use]
pub fn fingerprint(certificate: &CertificateDer<'_>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(certificate.as_ref());
    let mut out = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// Asks the certificate whether it is valid for `name`. A certificate that cannot be read and a name that is not a
/// DNS name both answer no: either way the credential does not carry that name.
fn valid_for(certificate: &CertificateDer<'_>, name: &str) -> bool {
    let Ok(end_entity) = webpki::EndEntityCert::try_from(certificate) else {
        return false;
    };
    let Ok(subject) = ServerName::try_from(name) else {
        return false;
    };
    end_entity
        .verify_is_valid_for_subject_name(&subject)
        .is_ok()
}

#[cfg(test)]
#[path = "credential_tests.rs"]
mod tests;
