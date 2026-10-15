use rustls::pki_types::CertificateDer;
use tessari_s3_types::{ErrorCategory, NodeId};

use super::{Refused, fingerprint, names, speaks_for, speaks_for_itself};
use crate::Error;
use crate::tls::InternalTls;

fn node(id: &str) -> NodeId {
    NodeId::new(id).expect("a valid node id")
}

/// A certificate issued for `name` alone, self-signed: the name is all these tests ask about, and whether a chain
/// is trusted is the TLS stack's question, asked before this one.
fn issued_for(name: &str) -> CertificateDer<'static> {
    let key = rcgen::KeyPair::generate().expect("a key");
    let params =
        rcgen::CertificateParams::new(vec![name.to_owned()]).expect("certificate parameters");
    params
        .self_signed(&key)
        .expect("a certificate")
        .der()
        .clone()
}

#[test]
fn a_peer_credential_names_its_node_and_the_link_it_was_issued_for() {
    assert_eq!(names(&node("n1")), "n1.peer.tessari-s3");
}

#[test]
fn a_certificate_issued_for_the_claimed_node_speaks_for_it() {
    let n1 = node("n1");
    let certificate = issued_for(&names(&n1));
    assert_eq!(speaks_for(Some(&certificate), &n1), Ok(()));
}

#[test]
fn a_certificate_issued_for_another_node_is_refused_with_its_own_fingerprint() {
    let certificate = issued_for(&names(&node("n2")));
    let refused = speaks_for(Some(&certificate), &node("n1"))
        .expect_err("n2's certificate does not speak for n1");
    assert_eq!(
        refused,
        Refused::NamesAnother {
            claimed: "n1".to_owned(),
            fingerprint: fingerprint(&certificate),
        }
    );
}

#[test]
fn a_certificate_issued_for_another_link_does_not_answer_for_this_one() {
    let n1 = node("n1");
    // The same node issued for a client link, and for the database's own peer link: neither is this link's name.
    for name in ["n1.client.tessari-s3", "n1.peer.tessari"] {
        let certificate = issued_for(name);
        assert!(
            matches!(
                speaks_for(Some(&certificate), &n1),
                Err(Refused::NamesAnother { .. })
            ),
            "{name} answered for n1's peer link"
        );
    }
}

#[test]
fn a_connection_that_presented_nothing_is_refused_as_unidentified() {
    assert_eq!(speaks_for(None, &node("n1")), Err(Refused::Unidentified));
}

#[test]
fn bytes_that_are_not_a_certificate_are_refused_rather_than_panicking() {
    let garbage = CertificateDer::from(vec![0_u8; 8]);
    assert!(matches!(
        speaks_for(Some(&garbage), &node("n1")),
        Err(Refused::NamesAnother { .. })
    ));
}

#[test]
fn a_fingerprint_is_the_sha256_of_the_der_in_lowercase_hex() {
    // SHA-256 of the empty input, a published constant.
    let empty = CertificateDer::from(Vec::new());
    assert_eq!(
        fingerprint(&empty),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn a_refusal_is_forbidden_rather_than_a_fault() {
    assert_eq!(Refused::Unidentified.category(), ErrorCategory::Forbidden);
}

/// This node's TLS material with a certificate issued for `name`, and that certificate's fingerprint.
fn material_for(name: &str) -> (InternalTls, String) {
    let key = rcgen::KeyPair::generate().expect("a key");
    let certificate = rcgen::CertificateParams::new(vec![name.to_owned()])
        .expect("certificate parameters")
        .self_signed(&key)
        .expect("a certificate");
    let tls = InternalTls::new(
        certificate.pem().into_bytes(),
        key.serialize_pem().into_bytes(),
        certificate.pem().into_bytes(),
    );
    (tls, fingerprint(certificate.der()))
}

#[test]
fn a_node_whose_own_certificate_is_another_nodes_does_not_start() {
    let n1 = node("n1");
    let (own, print) = material_for("n1.peer.tessari-s3");
    assert_eq!(
        speaks_for_itself(&own, &n1),
        Ok(print),
        "the fingerprint of its own certificate"
    );
    let refused = Err(Error::InvalidConfig {
        key: "TESSARIDB_S3_INTERNAL_TLS_CERT",
        reason: "issued for another node than TESSARIDB_S3_NODE_ID",
    });
    assert_eq!(
        speaks_for_itself(&material_for("n2.peer.tessari-s3").0, &n1),
        refused
    );
    assert_eq!(
        speaks_for_itself(&material_for("n1.client.tessari-s3").0, &n1),
        refused
    );
}
