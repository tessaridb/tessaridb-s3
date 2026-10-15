use super::{Need, required, required_naming_version};
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
fn exactly_the_operations_of_uploading_one_key_are_uploads() {
    let uploads: Vec<Operation> = implemented()
        .iter()
        .copied()
        .filter(|operation| required(*operation).is_some_and(|needs| needs.upload))
        .collect();
    assert_eq!(
        uploads,
        vec![
            Operation::AbortMultipartUpload,
            Operation::CompleteMultipartUpload,
            Operation::CreateMultipartUpload,
            Operation::ListParts,
            Operation::PutObject,
            Operation::UploadPart,
        ]
    );
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
fn writing_a_replica_under_its_source_version_id_needs_who_administers_the_bucket() {
    assert_eq!(
        required_naming_version(Operation::PutObject).map(|needs| needs.primary),
        Some(Need::ConfigureBucket)
    );
    assert_eq!(
        required(Operation::PutObject).map(|needs| needs.primary),
        Some(Need::WriteObject),
        "control: an ordinary write needs write access"
    );
}

#[test]
fn deleting_a_version_needs_who_administers_the_bucket() {
    assert_eq!(
        required_naming_version(Operation::DeleteObject).map(|needs| needs.primary),
        Some(Need::ConfigureBucket)
    );
    for read in [Operation::GetObject, Operation::HeadObject] {
        assert_eq!(
            required_naming_version(read).map(|needs| needs.primary),
            Some(Need::ReadObject),
            "{read:?}: reading a version is a read"
        );
    }
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
