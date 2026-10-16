use super::{KeyName, Sealing};
use crate::encryption::WrappedKey;

fn wrapped_by(kek: &str) -> WrappedKey {
    WrappedKey {
        algorithm: "xchacha20poly1305".to_owned(),
        kek_id: kek.to_owned(),
        nonce: vec![0; 24],
        ciphertext: vec![0; 48],
        customer: None,
    }
}

#[test]
fn a_key_name_is_lowercase_letters_digits_and_dashes_and_nothing_else() {
    for usable in ["a", "aws-s3", "tenant-7", &"x".repeat(64)] {
        assert_eq!(
            KeyName::new(usable).map(|name| name.as_str().to_owned()),
            Some(usable.to_owned())
        );
    }
    for refused in [
        "",
        "Tenant",
        "a_b",
        "a b",
        "a:b",
        "a'b",
        "a/b",
        "a;DROP",
        &"x".repeat(65),
    ] {
        assert_eq!(KeyName::new(refused), None, "{refused:?}");
    }
    assert_eq!(
        KeyName::new("aws-s3").expect("usable").record_id(),
        "kms_aws-s3"
    );
}

#[test]
fn the_key_that_wrapped_a_data_key_says_how_it_was_sealed() {
    assert_eq!(Sealing::of(&wrapped_by("server_1")), Some(Sealing::Server));
    assert_eq!(Sealing::of(&wrapped_by("server_12")), Some(Sealing::Server));
    assert_eq!(
        Sealing::of(&wrapped_by("kms_tenant-7")),
        Some(Sealing::Named(KeyName::new("tenant-7").expect("usable")))
    );
    for unknown in ["server_", "server_x", "kms_", "kms_Bad", "other_1", ""] {
        assert_eq!(Sealing::of(&wrapped_by(unknown)), None, "{unknown:?}");
    }
}
