use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::Value;

use super::{read, record};
use crate::Error;
use crate::encryption::WrappedKey;
use crate::objects::Lock;
use crate::objects::model::{Content, NewObject};

fn object(encryption: Option<WrappedKey>) -> NewObject {
    NewObject {
        size: 5,
        etag: "\"e\"".to_owned(),
        headers: BTreeMap::new(),
        metadata: BTreeMap::new(),
        tags: BTreeMap::new(),
        checksums: BTreeMap::new(),
        content: Content::Data([3; 16]),
        copied: false,
        lock: Lock::default(),
        encryption,
    }
}

fn wrapped() -> WrappedKey {
    WrappedKey {
        algorithm: "xchacha20poly1305".to_owned(),
        kek_id: "server_2".to_owned(),
        nonce: vec![4; 24],
        ciphertext: vec![5; 48],
    }
}

/// `object`'s record as the store hands it back: with the store's clock.
fn stored(object: &NewObject) -> Value {
    let Value::Object(mut fields) = record("b", "k", [1; 16], object).expect("record") else {
        panic!("a record is an object");
    };
    fields.insert(
        "modified".to_owned(),
        Value::Datetime {
            seconds: 1,
            nanos: 0,
        },
    );
    Value::Object(fields)
}

#[test]
fn a_sealed_object_keeps_its_wrapped_key_and_a_plain_one_carries_none() {
    let sealed = stored(&object(Some(wrapped())));
    let Value::Object(fields) = &sealed else {
        panic!("object")
    };
    assert_eq!(
        fields.get("sse_kek"),
        Some(&Value::String("server_2".to_owned()))
    );
    assert_eq!(fields.get("sse_key"), Some(&Value::Bytes(vec![5; 48])));
    assert_eq!(read(&sealed).expect("read").1.encryption, Some(wrapped()));

    let plain = stored(&object(None));
    let Value::Object(fields) = &plain else {
        panic!("object")
    };
    assert!(
        fields.keys().all(|name| !name.starts_with("sse")),
        "a plain record is what it always was: {fields:?}"
    );
    assert_eq!(read(&plain).expect("read").1.encryption, None);
}

#[test]
fn a_wrapped_key_missing_a_part_is_refused_rather_than_read_as_plain() {
    let Value::Object(mut fields) = stored(&object(Some(wrapped()))) else {
        panic!("object")
    };
    fields.remove("sse_nonce");
    assert!(matches!(
        read(&Value::Object(fields)),
        Err(Error::Malformed {
            record: "object",
            ..
        })
    ));
}
