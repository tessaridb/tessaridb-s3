use std::collections::BTreeMap;

use super::{
    Access, Action, BucketResource, Decision, Denied, Principal, Role, SpaceName, UploadPrincipal,
    UserPrincipal, Visible, authorize,
};

fn space(name: &str) -> SpaceName {
    SpaceName::new(name).expect("a valid space name")
}

fn member(grants: &[(&str, Access)]) -> UserPrincipal {
    UserPrincipal {
        name: "ann".to_owned(),
        space: space("alpha"),
        role: Role::Member,
        create_buckets: false,
        operator: false,
        cluster_viewer: false,
        grants: grants
            .iter()
            .map(|(bucket, access)| ((*bucket).to_owned(), *access))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn bucket(name: &str, in_space: &str) -> BucketResource {
    BucketResource {
        name: name.to_owned(),
        space: space(in_space),
        creator: None,
    }
}

fn upload_key(parent: UserPrincipal, bucket: &str, key: &str) -> Principal {
    Principal::Upload(UploadPrincipal {
        parent,
        bucket: bucket.to_owned(),
        key: key.to_owned(),
    })
}

fn uploading(bucket: BucketResource, key: &str) -> Action {
    Action::Upload {
        bucket,
        key: key.to_owned(),
    }
}

const WRITE: Access = Access {
    read: false,
    write: true,
};
const READ: Access = Access {
    read: true,
    write: false,
};
const OUT_OF_SCOPE: Decision = Decision::Deny(Denied::OutOfScope);

#[test]
fn an_upload_key_uploads_its_one_key_and_no_other() {
    let key = upload_key(member(&[("media", WRITE)]), "media", "in/a.jpg");
    assert_eq!(
        authorize(&key, &uploading(bucket("media", "alpha"), "in/a.jpg")),
        Decision::Allow
    );
    for (bucket_name, object) in [
        ("media", "in/b.jpg"),
        ("media", "in/a.jpg.bak"),
        ("media", "in/"),
        ("media", "IN/A.JPG"),
        ("photos", "in/a.jpg"),
    ] {
        assert_eq!(
            authorize(&key, &uploading(bucket(bucket_name, "alpha"), object)),
            OUT_OF_SCOPE,
            "{bucket_name}/{object}"
        );
    }
}

#[test]
fn an_upload_key_does_nothing_but_upload_even_for_an_operator() {
    let mut parent = member(&[(
        "media",
        Access {
            read: true,
            write: true,
        },
    )]);
    parent.operator = true;
    parent.cluster_viewer = true;
    let key = upload_key(parent, "media", "a");
    let media = bucket("media", "alpha");
    for action in [
        Action::ListBuckets,
        Action::CreateBucket(space("alpha")),
        Action::DeleteBucket(media.clone()),
        Action::ReadBucket(media.clone()),
        Action::ReadObject(media.clone()),
        Action::WriteObject(media),
        Action::ViewCluster,
        Action::ManageSpace(space("alpha")),
        Action::Operate,
    ] {
        assert_eq!(authorize(&key, &action), OUT_OF_SCOPE, "{action:?}");
    }
}

#[test]
fn its_authority_is_the_intersection_with_its_parents() {
    // The parent may only read: the key, scoped to the very key, still may not write it.
    let read_only = upload_key(member(&[("media", READ)]), "media", "a");
    assert_eq!(
        authorize(&read_only, &uploading(bucket("media", "alpha"), "a")),
        Decision::Deny(Denied::NoGrant)
    );
    // The grant is gone: so is the key's authority.
    let revoked = upload_key(member(&[]), "media", "a");
    assert_eq!(
        authorize(&revoked, &uploading(bucket("media", "alpha"), "a")),
        Decision::Deny(Denied::NoGrant)
    );
    // A bucket of the same name in another space is another bucket.
    let key = upload_key(member(&[("media", WRITE)]), "media", "a");
    assert_eq!(
        authorize(&key, &uploading(bucket("media", "beta"), "a")),
        Decision::Deny(Denied::OtherSpace)
    );
}

/// Against a naive restatement of the rule over every combination: the key allows exactly where the scope matches
/// AND the parent alone may write the bucket — never where only one of the two holds.
#[test]
fn it_allows_exactly_where_scope_and_parent_both_allow() {
    let parents = [
        member(&[]),
        member(&[("media", READ)]),
        member(&[("media", WRITE)]),
        member(&[("photos", WRITE)]),
        UserPrincipal {
            role: Role::SpaceAdmin,
            ..member(&[])
        },
    ];
    let buckets = [
        bucket("media", "alpha"),
        bucket("photos", "alpha"),
        bucket("media", "beta"),
    ];
    let mut allowed = 0_u32;
    for parent in &parents {
        for target in &buckets {
            for object in ["a", "b"] {
                let key = upload_key(parent.clone(), "media", "a");
                let in_scope = target.name == "media" && object == "a";
                let parent_may = authorize(
                    &Principal::User(parent.clone()),
                    &Action::WriteObject(target.clone()),
                ) == Decision::Allow;
                let decided = authorize(&key, &uploading(target.clone(), object));
                assert_eq!(
                    decided == Decision::Allow,
                    in_scope && parent_may,
                    "{parent:?} {target:?} {object}"
                );
                if decided == Decision::Allow {
                    allowed = allowed.saturating_add(1);
                }
            }
        }
    }
    // Control: the comparison above saw both answers, so it is not vacuous.
    assert_eq!(
        allowed, 2,
        "the writer member and the space admin, each on media/a"
    );
}

#[test]
fn a_user_uploading_is_judged_as_writing_the_bucket() {
    let action = uploading(bucket("media", "alpha"), "a");
    assert_eq!(
        authorize(&Principal::User(member(&[("media", WRITE)])), &action),
        Decision::Allow
    );
    assert_eq!(
        authorize(&Principal::User(member(&[("media", READ)])), &action),
        Decision::Deny(Denied::NoGrant)
    );
}

#[test]
fn an_upload_key_lists_nothing() {
    let key = upload_key(member(&[("media", WRITE)]), "media", "a");
    assert_eq!(
        key.visible(),
        Visible::Reachable {
            space: space("alpha"),
            user: String::new(),
            granted: Vec::new(),
        }
    );
}
