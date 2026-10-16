use zeroize::Zeroizing;

use super::{KeyRing, KeyState, Passphrase};
use crate::Error;
use crate::keys::entity::StoredKey;

fn passphrase(text: &str) -> crate::Result<Passphrase> {
    Passphrase::new(Zeroizing::new(text.to_owned()))
}

fn stored(id: &str, byte: u8) -> StoredKey {
    StoredKey {
        id: id.to_owned(),
        key: Zeroizing::new([byte; 32]),
    }
}

#[test]
fn a_passphrase_that_would_need_escaping_is_refused_before_anything_is_sent() {
    for text in [
        "fifteen chars quote'here",
        "fifteen chars back\\slash",
        "fifteen chars new\nline",
        "fourteen chars",
    ] {
        assert!(
            matches!(passphrase(text), Err(Error::WeakPassphrase { .. })),
            "{text:?}"
        );
    }
    let long: String = "é".repeat(257);
    assert!(matches!(
        passphrase(&long),
        Err(Error::WeakPassphrase { .. })
    ));
    // Counted in characters, not bytes: fifteen two-byte characters pass.
    let accepted = passphrase(&"é".repeat(15)).expect("fifteen characters");
    assert_eq!(accepted.expose(), "é".repeat(15));
    assert_eq!(format!("{accepted:?}"), "Passphrase(..)");
}

#[test]
fn the_active_key_is_the_highest_numbered_server_key() {
    let ring = KeyRing::new(vec![
        stored("server_2", 2),
        stored("server_10", 10),
        stored("server_9", 9),
        stored("kms_billing", 11),
    ])
    .expect("a server key");
    assert_eq!(
        ring.active().id(),
        "server_10",
        "numbered, not sorted as text"
    );
    assert!(ring.get("server_2").is_some(), "an older key still unwraps");
    assert!(ring.get("kms_billing").is_some());
    assert!(ring.get("server_3").is_none());
    assert_eq!(
        ring.state(),
        KeyState::Unsealed {
            active: "server_10".to_owned(),
            keys: 4,
            named: vec!["billing".to_owned()],
        }
    );
    assert!(!format!("{ring:?}").contains('['), "no key bytes printed");
}

#[test]
fn a_ring_with_no_server_key_is_refused() {
    for keys in [
        vec![],
        vec![stored("kms_x", 1)],
        vec![stored("server_x", 1)],
    ] {
        assert!(matches!(KeyRing::new(keys), Err(Error::Unsealable)));
    }
}
