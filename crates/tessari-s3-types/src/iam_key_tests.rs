use super::IamKey;

const HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn counting() -> [u8; 32] {
    let mut bytes = [0_u8; 32];
    for (value, byte) in (0_u8..).zip(bytes.iter_mut()) {
        *byte = value;
    }
    bytes
}

#[test]
fn sixty_four_hex_digits_are_the_key_in_either_case() {
    let lower = IamKey::from_hex(HEX).expect("a key");
    assert_eq!(lower.expose(), &counting());
    let upper = IamKey::from_hex(&format!("  {}\n", HEX.to_ascii_uppercase())).expect("a key");
    assert_eq!(upper.expose(), &counting());
}

#[test]
fn anything_but_sixty_four_hex_digits_is_refused() {
    for bad in [
        "",
        &HEX[..62],
        &format!("{HEX}00"),
        &format!("{}zz", &HEX[..62]),
        &format!("{} {}", &HEX[..32], &HEX[32..]),
    ] {
        assert!(IamKey::from_hex(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn the_key_never_appears_in_debug_output() {
    let key = IamKey::from_hex(&"ab".repeat(32)).expect("a key");
    assert_eq!(format!("{key:?}"), "IamKey(..)");
}
