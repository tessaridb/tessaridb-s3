use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use openidconnect::JsonWebKey;
use openidconnect::core::CoreJwsSigningAlgorithm;
use sha2::{Digest, Sha256};

use super::ClientKey;

const ISSUER: &str = "https://id.example.test/acme";

fn ed25519_pem() -> String {
    rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .expect("an Ed25519 key")
        .serialize_pem()
}

fn part(token: &str, index: usize) -> serde_json::Value {
    let raw = token.split('.').nth(index).expect("three parts");
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(raw).expect("base64url")).expect("JSON")
}

#[test]
fn an_assertion_names_the_client_addresses_the_issuer_and_verifies_under_the_public_key() {
    let key = ClientKey::from_pem(&ed25519_pem()).expect("an Ed25519 key loads");
    let token = key
        .assertion("tessaridb-s3-console", ISSUER, 1_000_000)
        .expect("signs");
    let header = part(&token, 0);
    assert_eq!(header["alg"], "EdDSA");
    assert_eq!(header["typ"], "JWT");
    assert_eq!(header["kid"], key.kid());
    let claims = part(&token, 1);
    assert_eq!(claims["iss"], "tessaridb-s3-console");
    assert_eq!(claims["sub"], "tessaridb-s3-console");
    assert_eq!(claims["aud"], ISSUER);
    assert_eq!(claims["iat"], 1_000_000);
    assert_eq!(claims["exp"], 1_000_060);
    assert!(claims["jti"].as_str().is_some_and(|jti| jti.len() >= 16));
    let (signed, signature) = token.rsplit_once('.').expect("a signature");
    let public: openidconnect::core::CoreJsonWebKey =
        serde_json::from_str(&key.public_jwk()).expect("the public JWK parses");
    public
        .verify_signature(
            &CoreJwsSigningAlgorithm::EdDsa,
            signed.as_bytes(),
            &URL_SAFE_NO_PAD.decode(signature).expect("base64url"),
        )
        .expect("the signature verifies under the published key");
}

#[test]
fn every_assertion_carries_its_own_jti() {
    let key = ClientKey::from_pem(&ed25519_pem()).expect("loads");
    let first = part(&key.assertion("c", ISSUER, 10).expect("signs"), 1);
    let second = part(&key.assertion("c", ISSUER, 10).expect("signs"), 1);
    assert_ne!(first["jti"], second["jti"]);
}

#[test]
fn the_key_id_is_the_rfc_7638_thumbprint_and_the_jwk_holds_no_private_part() {
    let key = ClientKey::from_pem(&ed25519_pem()).expect("loads");
    let jwk: serde_json::Value = serde_json::from_str(&key.public_jwk()).expect("JSON");
    assert_eq!(jwk["kty"], "OKP");
    assert_eq!(jwk["crv"], "Ed25519");
    assert_eq!(jwk["alg"], "EdDSA");
    assert!(jwk.get("d").is_none());
    let x = jwk["x"].as_str().expect("x");
    let canonical = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{x}"}}"#);
    assert_eq!(
        key.kid(),
        URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes()))
    );
    assert_eq!(jwk["kid"], key.kid());
}

#[test]
fn a_key_that_is_not_ed25519_is_refused() {
    let p256 = rcgen::KeyPair::generate()
        .expect("a P-256 key")
        .serialize_pem();
    assert!(ClientKey::from_pem(&p256).is_err());
    assert!(ClientKey::from_pem("not a key").is_err());
}

#[test]
fn the_debug_form_shows_the_key_id_and_nothing_private() {
    let pem = ed25519_pem();
    let key = ClientKey::from_pem(&pem).expect("loads");
    let shown = format!("{key:?}");
    assert!(shown.contains(key.kid()));
    let body = pem.lines().nth(1).expect("a PEM body line");
    assert!(!shown.contains(body));
}
