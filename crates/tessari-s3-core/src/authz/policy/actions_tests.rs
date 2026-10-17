//! The action table beside the dispatch catalog: an implemented operation a bucket policy could govern cannot ship
//! without the action it is asked as, and every action the table asks is one a policy can name.

use super::{S3_ACTIONS, Scope, policy_action};
use crate::dispatch::{Operation, implemented};

#[test]
fn every_implemented_bucket_operation_is_asked_as_a_known_action() {
    let outside = [Operation::ListBuckets, Operation::CreateBucket];
    let missing: Vec<Operation> = implemented()
        .iter()
        .copied()
        .filter(|operation| {
            !outside.contains(operation) && policy_action(*operation, false).is_none()
        })
        .collect();
    assert_eq!(missing, Vec::<Operation>::new());
    for operation in implemented() {
        for naming_version in [false, true] {
            if let Some((action, _)) = policy_action(*operation, naming_version) {
                let name = action.strip_prefix("s3:").expect("an s3 action");
                assert!(S3_ACTIONS.contains(&name), "{operation:?} asks {action}");
            }
        }
    }
}

#[test]
fn reading_or_deleting_a_named_version_is_its_own_action() {
    assert_eq!(
        policy_action(Operation::GetObject, true),
        Some(("s3:GetObjectVersion", Scope::Object))
    );
    assert_eq!(
        policy_action(Operation::DeleteObject, true),
        Some(("s3:DeleteObjectVersion", Scope::Object))
    );
    assert_eq!(
        policy_action(Operation::ListObjectsV2, false),
        Some(("s3:ListBucket", Scope::Bucket))
    );
}
