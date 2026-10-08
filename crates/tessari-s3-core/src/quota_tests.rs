use crate::authz::{Need, required};
use crate::dispatch::{Operation, is_implemented};
use crate::quota::{ADDING, Adding, Exceeded, Held, Quota, adds_to_bucket, admits};

const HELD: Held = Held {
    objects: 9,
    bytes: 900,
};

const fn adding(bytes: u64, object: bool) -> Adding {
    Adding { bytes, object }
}

#[test]
fn reaching_a_limit_is_admitted_and_passing_it_is_not() {
    let bytes = Quota {
        max_bytes: Some(1000),
        max_objects: None,
    };
    assert_eq!(admits(bytes, HELD, adding(100, true)), Ok(()));
    assert_eq!(admits(bytes, HELD, adding(101, true)), Err(Exceeded::Bytes));
    let objects = Quota {
        max_bytes: None,
        max_objects: Some(10),
    };
    assert_eq!(admits(objects, HELD, adding(5_000, true)), Ok(()));
    let full = Held {
        objects: 10,
        bytes: 0,
    };
    assert_eq!(
        admits(objects, full, adding(1, true)),
        Err(Exceeded::Objects)
    );
    assert_eq!(
        admits(objects, full, adding(1, false)),
        Ok(()),
        "a part or an overwrite adds no object"
    );
}

#[test]
fn no_limit_admits_anything_and_needs_no_measurement() {
    assert!(!Quota::default().limits());
    assert_eq!(
        admits(Quota::default(), HELD, adding(u64::MAX, true)),
        Ok(())
    );
}

#[test]
fn a_sum_that_would_overflow_is_over_the_limit() {
    let quota = Quota {
        max_bytes: Some(u64::MAX - 1),
        max_objects: None,
    };
    let held = Held {
        objects: 0,
        bytes: u64::MAX - 1,
    };
    assert_eq!(
        admits(quota, held, adding(u64::MAX, false)),
        Err(Exceeded::Bytes)
    );
}

#[test]
fn the_adding_operations_are_implemented_writes_and_only_they_add() {
    for operation in ADDING {
        assert!(is_implemented(operation), "{operation:?}");
        assert_eq!(
            required(operation).map(|needs| needs.primary),
            Some(Need::WriteObject),
            "{operation:?}"
        );
        assert!(adds_to_bucket(operation));
    }
    assert_eq!(ADDING.len(), 5);
    for removes in [
        Operation::DeleteObject,
        Operation::DeleteObjects,
        Operation::AbortMultipartUpload,
        Operation::CreateMultipartUpload,
    ] {
        assert!(!adds_to_bucket(removes), "{removes:?}");
    }
}
