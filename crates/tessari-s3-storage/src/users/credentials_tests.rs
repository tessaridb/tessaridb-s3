use super::{access_key_id, is_access_key_id};

#[test]
fn every_issued_id_has_the_issued_shape() {
    for _ in 0..64 {
        let id = access_key_id().expect("random");
        assert!(is_access_key_id(&id), "{id}");
    }
}

#[test]
fn any_other_shape_was_never_issued() {
    for other in [
        "",
        "AKIAIOSFODNN7EXAMPLE",
        "TS",
        "TSAAAAAAAAAAAAAAAAA",
        "TSAAAAAAAAAAAAAAAAAAA",
        "TSAAAAAAAAAAAAAAAAA1",
        "TSAAAAAAAAAAAAAAAAAa",
        "tsAAAAAAAAAAAAAAAAAA",
        "TSAAAAAAAAAAAAAAAAA/",
    ] {
        assert!(!is_access_key_id(other), "{other:?}");
    }
}
