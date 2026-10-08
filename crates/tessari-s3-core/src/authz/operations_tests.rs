use super::{Need, required};
use crate::dispatch::{Operation, implemented};

#[test]
fn every_implemented_operation_has_a_row() {
    let missing: Vec<Operation> = implemented()
        .iter()
        .copied()
        .filter(|operation| required(*operation).is_none())
        .collect();
    assert_eq!(missing, Vec::<Operation>::new());
}

#[test]
fn copies_also_need_the_source_readable() {
    for operation in [Operation::CopyObject, Operation::UploadPartCopy] {
        let needs = required(operation).expect("copy operations are implemented");
        assert_eq!(needs.primary, Need::WriteObject);
        assert!(needs.source_read, "{operation:?}");
    }
    let put = required(Operation::PutObject).expect("PutObject is implemented");
    assert!(!put.source_read);
}

#[test]
fn deleting_objects_needs_write_and_reading_needs_read() {
    assert_eq!(
        required(Operation::DeleteObjects).map(|needs| needs.primary),
        Some(Need::WriteObject)
    );
    assert_eq!(
        required(Operation::GetObject).map(|needs| needs.primary),
        Some(Need::ReadObject)
    );
    assert_eq!(
        required(Operation::ListObjectsV2).map(|needs| needs.primary),
        Some(Need::ReadBucket)
    );
}
