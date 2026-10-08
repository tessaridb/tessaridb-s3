use tessari_s3_types::{IamKey, SecretKey};

use super::{Binding, Sealed, Sealer};
use crate::Error;

const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

fn sealer(fill: &str) -> Sealer {
    Sealer::new(IamKey::from_hex(&fill.repeat(32)).expect("a key"))
}

const fn binding() -> Binding<'static> {
    Binding {
        key_id: "TSAAAAAAAAAAAAAAAAAA",
        user: "ann",
        space: "alpha",
    }
}

fn secret() -> SecretKey {
    SecretKey::new(SECRET.to_owned())
}

fn refused(sealer: &Sealer, sealed: &Sealed, binding: Binding<'_>) -> bool {
    matches!(sealer.open(sealed, binding), Err(Error::Unsealable))
}

#[test]
fn a_sealed_secret_opens_where_it_was_sealed() {
    let sealer = sealer("a1");
    let sealed = sealer.seal(&secret(), binding()).expect("sealed");
    assert_eq!(sealed.algorithm, "xchacha20poly1305");
    assert_eq!(sealed.kek_id, sealer.kek_id);
    assert_eq!(sealed.nonce.len(), 24);
    let opened = sealer.open(&sealed, binding()).expect("opens");
    assert_eq!(opened.expose(), SECRET);
}

#[test]
fn the_stored_bytes_are_not_the_secret_and_never_repeat() {
    let sealer = sealer("a1");
    let first = sealer.seal(&secret(), binding()).expect("sealed");
    let second = sealer.seal(&secret(), binding()).expect("sealed");
    assert_ne!(first.nonce, second.nonce);
    assert_ne!(first.ciphertext, second.ciphertext);
    for sealed in [&first, &second] {
        assert_eq!(
            sealed.ciphertext.len(),
            SECRET.len() + 16,
            "ciphertext and tag"
        );
        assert!(
            !sealed
                .ciphertext
                .windows(8)
                .any(|window| SECRET.as_bytes().windows(8).any(|part| part == window)),
            "no 8 bytes of the secret appear in the stored bytes"
        );
    }
}

#[test]
fn the_root_key_id_is_stable_and_tells_two_roots_apart() {
    assert_eq!(sealer("a1").kek_id, sealer("a1").kek_id);
    assert_ne!(sealer("a1").kek_id, sealer("b2").kek_id);
    assert_eq!(sealer("a1").kek_id.len(), 16, "eight bytes, hex");
}

#[test]
fn another_root_does_not_open_it_even_when_the_id_is_forged() {
    let sealed = sealer("a1").seal(&secret(), binding()).expect("sealed");
    let other = sealer("b2");
    assert!(refused(&other, &sealed, binding()));
    let forged = Sealed {
        kek_id: other.kek_id.clone(),
        ..sealed
    };
    assert!(refused(&other, &forged, binding()));
}

#[test]
fn a_changed_byte_does_not_open() {
    let sealer = sealer("a1");
    let sealed = sealer.seal(&secret(), binding()).expect("sealed");
    let mut ciphertext = sealed.clone();
    if let Some(byte) = ciphertext.ciphertext.get_mut(3) {
        *byte ^= 1;
    }
    assert!(refused(&sealer, &ciphertext, binding()));
    let mut nonce = sealed;
    if let Some(byte) = nonce.nonce.get_mut(0) {
        *byte ^= 1;
    }
    assert!(refused(&sealer, &nonce, binding()));
}

#[test]
fn a_secret_moved_to_another_key_user_or_space_does_not_open() {
    let sealer = sealer("a1");
    let sealed = sealer.seal(&secret(), binding()).expect("sealed");
    let elsewhere = [
        Binding {
            key_id: "TSBBBBBBBBBBBBBBBBBB",
            ..binding()
        },
        Binding {
            user: "bob",
            ..binding()
        },
        Binding {
            space: "beta",
            ..binding()
        },
    ];
    for moved in elsewhere {
        assert!(refused(&sealer, &sealed, moved), "{moved:?}");
    }
}

#[test]
fn an_unknown_algorithm_or_a_short_nonce_does_not_open() {
    let sealer = sealer("a1");
    let sealed = sealer.seal(&secret(), binding()).expect("sealed");
    let algorithm = Sealed {
        algorithm: "aes-256-gcm".to_owned(),
        ..sealed.clone()
    };
    assert!(refused(&sealer, &algorithm, binding()));
    let mut nonce = sealed;
    nonce.nonce.truncate(12);
    assert!(refused(&sealer, &nonce, binding()));
}

#[test]
fn everything_stored_but_the_root_opens_nothing() {
    // An attacker holding the metadata store knows the record, its binding and the root's id — everything but the
    // root itself. A sealer built over another root and given that id must still refuse.
    let sealed = sealer("a1").seal(&secret(), binding()).expect("sealed");
    let mut attacker = sealer("b2");
    attacker.kek_id = sealed.kek_id.clone();
    assert!(refused(&attacker, &sealed, binding()));
}
