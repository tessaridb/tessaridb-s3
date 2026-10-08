//! The internal surface's TLS. One cluster authority issues every node a certificate for the host it advertises; a
//! node presents it as a server and as a client, and accepts only peers that authority issued. Ring is the provider,
//! as for the metadata client, and only TLS 1.2 and 1.3 are spoken. There is no switch that skips verification.
//! The configurations are immutable once built and shared across connections behind an `Arc`.

use std::sync::Arc;

use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig};

use crate::{Error, Result};

/// The internal surface's TLS material, read at start: this node's certificate chain (valid for the host it
/// advertises), its private key, and the cluster's certificate authority. The key is never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct InternalTls {
    /// This node's certificate chain, PEM.
    pub certificate_pem: Vec<u8>,
    key_pem: Vec<u8>,
    /// The cluster's certificate authority, PEM.
    pub authority_pem: Vec<u8>,
}

impl InternalTls {
    /// The material as read.
    #[must_use]
    pub const fn new(certificate_pem: Vec<u8>, key_pem: Vec<u8>, authority_pem: Vec<u8>) -> Self {
        Self {
            certificate_pem,
            key_pem,
            authority_pem,
        }
    }

    /// This node's private key, PEM.
    #[must_use]
    pub fn key_pem(&self) -> &[u8] {
        &self.key_pem
    }
}

impl std::fmt::Debug for InternalTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InternalTls")
            .field("certificate_pem", &self.certificate_pem.len())
            .field("key_pem", &"<redacted>")
            .field("authority_pem", &self.authority_pem.len())
            .finish()
    }
}

const CERT: &str = "TESSARIDB_S3_INTERNAL_TLS_CERT";
const KEY: &str = "TESSARIDB_S3_INTERNAL_TLS_KEY";
const CA: &str = "TESSARIDB_S3_INTERNAL_TLS_CA";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

const fn invalid(key: &'static str, reason: &'static str) -> Error {
    Error::InvalidConfig { key, reason }
}

fn certificates(pem: &[u8], key: &'static str) -> Result<Vec<CertificateDer<'static>>> {
    let chain = CertificateDer::pem_slice_iter(pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| invalid(key, "not PEM certificates"))?;
    if chain.is_empty() {
        return Err(invalid(key, "holds no certificate"));
    }
    Ok(chain)
}

fn private_key(tls: &InternalTls) -> Result<PrivateKeyDer<'static>> {
    PrivateKeyDer::from_pem_slice(tls.key_pem()).map_err(|_| invalid(KEY, "not a PEM private key"))
}

fn authority(tls: &InternalTls) -> Result<Arc<RootCertStore>> {
    let mut roots = RootCertStore::empty();
    for certificate in certificates(&tls.authority_pem, CA)? {
        roots
            .add(certificate)
            .map_err(|_| invalid(CA, "not a usable certificate authority"))?;
    }
    Ok(Arc::new(roots))
}

/// The surface's side: this node's certificate, and a client certificate the cluster authority issued required of
/// every connection before a request is read.
///
/// # Errors
/// [`Error::InvalidConfig`] naming the file whose contents cannot be used.
pub fn server_config(tls: &InternalTls) -> Result<Arc<ServerConfig>> {
    let verifier = WebPkiClientVerifier::builder_with_provider(authority(tls)?, provider())
        .build()
        .map_err(|_| invalid(CA, "not a usable certificate authority"))?;
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|_| invalid(CERT, "no protocol version both sides speak"))?
        .with_client_cert_verifier(verifier)
        .with_single_cert(certificates(&tls.certificate_pem, CERT)?, private_key(tls)?)
        .map_err(|_| invalid(KEY, "does not belong to the certificate"))?;
    Ok(Arc::new(config))
}

/// A caller in the clear: no authority is trusted, so nothing can be reached over TLS with it. It exists because
/// the HTTP client needs a TLS stack with a provider even when it speaks only plain HTTP.
///
/// # Errors
/// [`Error::InvalidConfig`] when no protocol version is available.
pub fn plain_client_config() -> Result<ClientConfig> {
    Ok(ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|_| invalid(CERT, "no protocol version both sides speak"))?
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth())
}

/// The caller's side: the peer must present a certificate the cluster authority issued for the host dialled, and
/// this node presents its own.
///
/// # Errors
/// [`Error::InvalidConfig`] naming the file whose contents cannot be used.
pub fn client_config(tls: &InternalTls) -> Result<ClientConfig> {
    ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|_| invalid(CERT, "no protocol version both sides speak"))?
        .with_root_certificates(authority(tls)?)
        .with_client_auth_cert(certificates(&tls.certificate_pem, CERT)?, private_key(tls)?)
        .map_err(|_| invalid(KEY, "does not belong to the certificate"))
}

/// The console's TLS material, read at start: its certificate chain and private key. Browsers verify the chain;
/// there is no client certificate. The key is never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct ConsoleTls {
    /// The console's certificate chain, PEM.
    pub certificate_pem: Vec<u8>,
    key_pem: Vec<u8>,
}

impl ConsoleTls {
    /// The material as read.
    #[must_use]
    pub const fn new(certificate_pem: Vec<u8>, key_pem: Vec<u8>) -> Self {
        Self {
            certificate_pem,
            key_pem,
        }
    }
}

impl std::fmt::Debug for ConsoleTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsoleTls")
            .field("certificate_pem", &self.certificate_pem.len())
            .field("key_pem", &"<redacted>")
            .finish()
    }
}

const CONSOLE_CERT: &str = "TESSARIDB_S3_CONSOLE_TLS_CERT";
const CONSOLE_KEY: &str = "TESSARIDB_S3_CONSOLE_TLS_KEY";

/// The console's side of TLS: its certificate, TLS 1.2 and 1.3 only, no client certificate asked for.
///
/// # Errors
/// [`Error::InvalidConfig`] naming the file whose contents cannot be used.
pub fn console_server_config(tls: &ConsoleTls) -> Result<Arc<ServerConfig>> {
    let key = PrivateKeyDer::from_pem_slice(&tls.key_pem)
        .map_err(|_| invalid(CONSOLE_KEY, "not a PEM private key"))?;
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|_| invalid(CONSOLE_CERT, "no protocol version both sides speak"))?
        .with_no_client_auth()
        .with_single_cert(certificates(&tls.certificate_pem, CONSOLE_CERT)?, key)
        .map_err(|_| invalid(CONSOLE_KEY, "does not belong to the certificate"))?;
    Ok(Arc::new(config))
}
