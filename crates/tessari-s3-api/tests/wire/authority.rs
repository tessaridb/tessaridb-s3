//! A certificate authority for one test cluster, issuing each node the certificate its peer credential needs.

use tessari_s3_infrastructure::InternalTls;

/// A certificate authority for one test cluster.
pub(crate) struct Authority {
    certificate: rcgen::Certificate,
    key: rcgen::KeyPair,
}

impl Authority {
    pub(crate) fn new() -> Self {
        let mut params = rcgen::CertificateParams::new(Vec::new()).expect("authority parameters");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().expect("authority key");
        let certificate = params.self_signed(&key).expect("authority");
        Self { certificate, key }
    }

    /// The TLS material of a node whose certificate this authority issued for `node`'s peer name.
    pub(crate) fn issue(&self, node: &str) -> InternalTls {
        let key = rcgen::KeyPair::generate().expect("node key");
        let leaf = rcgen::CertificateParams::new(vec![format!("{node}.peer.tessari-s3")])
            .expect("node parameters")
            .signed_by(&key, &self.certificate, &self.key)
            .expect("node certificate");
        InternalTls::new(
            leaf.pem().into_bytes(),
            key.serialize_pem().into_bytes(),
            self.certificate.pem().into_bytes(),
        )
    }
}
