use super::{
    BLOCK_TAG_LEN, DataKey, KeyEncryptionKey, WRAP_ALGORITHM, aead_seal, open_block, seal_block,
    unwrap, wrap,
};
use zeroize::Zeroizing;

use crate::Error;

fn hex(text: &str) -> Vec<u8> {
    let text: String = text.split_whitespace().collect();
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16).expect("hex"))
        .collect()
}

fn kek(id: &str, byte: u8) -> KeyEncryptionKey {
    KeyEncryptionKey::new(id.to_owned(), Zeroizing::new([byte; 32]))
}

const DATA: [u8; 16] = [7; 16];
const OTHER: [u8; 16] = [8; 16];

/// The AEAD every seal goes through, against draft-irtf-cfrg-xchacha-03 §A.3.1 (retrieved 2026-10-16 from
/// ietf.org/archive/id/draft-irtf-cfrg-xchacha-03.txt).
#[test]
fn the_aead_matches_the_published_xchacha20_poly1305_vector() {
    let key: [u8; 32] = hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f")
        .try_into()
        .expect("32 bytes");
    let nonce: [u8; 24] = hex("404142434445464748494a4b4c4d4e4f5051525354555657")
        .try_into()
        .expect("24 bytes");
    let plaintext = hex(
        "4c616469657320616e642047656e746c656d656e206f662074686520636c6173
         73206f66202739393a204966204920636f756c64206f6666657220796f75206f
         6e6c79206f6e652074697020666f7220746865206675747572652c2073756e73
         637265656e20776f756c642062652069742e",
    );
    let expected = hex(
        "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb
         731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b452
         2f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff9
         21f9664c97637da9768812f615c68b13b52e
         c0875924c1c7987947deafd8780acf49",
    );
    let sealed =
        aead_seal(&key, &nonce, &hex("50515253c0c1c2c3c4c5c6c7"), &plaintext).expect("sealed");
    assert_eq!(sealed, expected);
}

#[test]
fn a_block_opens_only_at_its_own_place_under_its_own_key() {
    let key = DataKey::generate().expect("a key");
    let plain = b"object bytes".to_vec();
    let sealed = seal_block(&key, DATA, 3, false, &plain).expect("sealed");
    assert_eq!(sealed.len(), plain.len() + BLOCK_TAG_LEN);
    assert_ne!(
        &sealed[..plain.len()],
        plain.as_slice(),
        "ciphertext, not the bytes"
    );
    assert_eq!(
        open_block(&key, DATA, 3, false, &sealed).expect("opens"),
        plain
    );

    let refused = |outcome: crate::Result<Vec<u8>>| matches!(outcome, Err(Error::Unsealable));
    assert!(
        refused(open_block(&key, DATA, 4, false, &sealed)),
        "another block index"
    );
    assert!(
        refused(open_block(&key, OTHER, 3, false, &sealed)),
        "another data file"
    );
    assert!(
        refused(open_block(&key, DATA, 3, true, &sealed)),
        "claimed as the last block: truncation"
    );
    let other = DataKey::generate().expect("a key");
    assert!(
        refused(open_block(&other, DATA, 3, false, &sealed)),
        "another data key"
    );
    let mut flipped = sealed.clone();
    flipped[0] ^= 1;
    assert!(
        refused(open_block(&key, DATA, 3, false, &flipped)),
        "a changed byte"
    );
    assert!(
        refused(open_block(
            &key,
            DATA,
            3,
            false,
            &sealed[..sealed.len() - 1]
        )),
        "cut short"
    );
}

#[test]
fn a_data_key_unwraps_only_under_its_kek_for_its_data_file() {
    let server = kek("server_1", 1);
    let key = DataKey::generate().expect("a key");
    let wrapped = wrap(&server, &key, DATA).expect("wrapped");
    assert_eq!(wrapped.algorithm, WRAP_ALGORITHM);
    assert_eq!(wrapped.kek_id, "server_1");
    let block = seal_block(&key, DATA, 0, true, b"x").expect("sealed");
    let back = unwrap(&server, &wrapped, DATA).expect("unwraps");
    assert_eq!(
        open_block(&back, DATA, 0, true, &block).expect("the same key"),
        b"x"
    );

    let refused = |outcome: crate::Result<DataKey>| matches!(outcome, Err(Error::Unsealable));
    assert!(
        refused(unwrap(&kek("server_1", 2), &wrapped, DATA)),
        "same id, other key"
    );
    assert!(
        refused(unwrap(&kek("server_2", 1), &wrapped, DATA)),
        "named for another kek"
    );
    assert!(
        refused(unwrap(&server, &wrapped, OTHER)),
        "moved to another data file"
    );
    let mut algorithm = wrapped.clone();
    algorithm.algorithm = "aes256gcm".to_owned();
    assert!(
        refused(unwrap(&server, &algorithm, DATA)),
        "an algorithm not on the list"
    );
    let mut tampered = wrapped.clone();
    tampered.ciphertext[0] ^= 1;
    assert!(refused(unwrap(&server, &tampered, DATA)), "a changed byte");
}

#[test]
fn every_key_and_every_wrap_is_fresh_and_no_key_is_printed() {
    let first = DataKey::generate().expect("a key");
    let second = DataKey::generate().expect("a key");
    let server = kek("server_1", 1);
    let one = wrap(&server, &first, DATA).expect("wrapped");
    let two = wrap(&server, &first, DATA).expect("wrapped");
    assert_ne!(one.nonce, two.nonce, "a random nonce per wrap");
    assert_ne!(one.ciphertext, two.ciphertext);
    let block = seal_block(&first, DATA, 0, true, b"same").expect("sealed");
    assert_ne!(
        block,
        seal_block(&second, DATA, 0, true, b"same").expect("sealed")
    );
    let shown = format!("{first:?} {server:?}");
    assert!(
        !shown.contains("01, 01") && !shown.contains("[1"),
        "{shown}"
    );
    assert!(shown.contains("server_1"), "the id is not secret: {shown}");
}
