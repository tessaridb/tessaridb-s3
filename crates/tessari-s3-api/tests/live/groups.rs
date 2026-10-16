//! Groups against a real node: a bucket granted to a group reaches every member and nobody else; taking a member out,
//! the grant away or the group itself ends the access once the member's resolution is older than the 5-second
//! window — measured on a clock the test moves — and a group never reaches across spaces.

use std::sync::atomic::Ordering;

use tessari_s3_core::authz::{Access, GroupName, Role};
use tessari_s3_storage::Storage;
use tessari_s3_storage::groups::MemberChange;
use tessari_s3_types::BucketName;

use crate::access::{Key, space, user_with_key};
use crate::access_window::state_with_clock;
use crate::{IGNORED, call_as};

const READ: Access = Access {
    read: true,
    write: false,
};

fn group(name: &str) -> GroupName {
    GroupName::new(name).expect("a group name")
}

/// Space `alpha` with `owner` holding bucket `shared` (one object), group `readers` granted it to read, `ann` a
/// member and `bob` not.
async fn readers(storage: &Storage, state: &tessari_s3_api::ApiState) -> (Key, Key) {
    let owner = user_with_key(storage, "owner", "alpha", Role::Member, true).await;
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, false).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    for (path, body) in [("/shared", &b""[..]), ("/shared/a.txt", &b"hello"[..])] {
        let made = call_as(state, owner.pair(), "PUT", path, vec![], vec![], body).await;
        assert_eq!(made.status, 200, "{path}: {}", made.body);
    }
    let groups = storage.groups();
    assert!(
        groups
            .create(&space("alpha"), &group("readers"))
            .await
            .expect("created")
    );
    let bucket = BucketName::new("shared").expect("a bucket name");
    assert_eq!(
        groups
            .set_grant(
                &space("alpha"),
                &group("readers"),
                &bucket,
                &space("alpha"),
                READ
            )
            .await
            .expect("granted"),
        MemberChange::Done
    );
    let user = tessari_s3_core::authz::UserName::new("ann").expect("a user name");
    assert_eq!(
        groups
            .add_member(&space("alpha"), &group("readers"), &user, &space("alpha"))
            .await
            .expect("added"),
        MemberChange::Done
    );
    (ann, bob)
}

async fn read(state: &tessari_s3_api::ApiState, key: &Key) -> (u16, Option<String>) {
    let seen = call_as(
        state,
        key.pair(),
        "GET",
        "/shared/a.txt",
        vec![],
        vec![],
        b"",
    )
    .await;
    (seen.status, seen.code)
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_group_grant_reaches_its_members_and_nobody_else() {
    let _why = IGNORED;
    let (state, _) = state_with_clock().await;
    let (ann, bob) = readers(state.storage(), &state).await;
    assert_eq!(
        read(&state, &ann).await.0,
        200,
        "a member reads through the group"
    );
    assert_eq!(
        read(&state, &bob).await,
        (403, Some("AccessDenied".to_owned())),
        "a non-member does not"
    );
    let write = call_as(
        &state,
        ann.pair(),
        "PUT",
        "/shared/b.txt",
        vec![],
        vec![],
        b"x",
    )
    .await;
    assert_eq!(
        (write.status, write.code.as_deref()),
        (403, Some("AccessDenied")),
        "the group grants reading only"
    );
    let listed = call_as(&state, ann.pair(), "GET", "/", vec![], vec![], b"").await;
    assert!(
        listed.body.contains("<Name>shared</Name>"),
        "{}",
        listed.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn two_groups_on_one_bucket_give_what_either_allows() {
    let _why = IGNORED;
    let (state, _) = state_with_clock().await;
    let (ann, _) = readers(state.storage(), &state).await;
    let groups = state.storage().groups();
    let bucket = BucketName::new("shared").expect("a bucket name");
    let user = tessari_s3_core::authz::UserName::new("ann").expect("a user name");
    assert!(
        groups
            .create(&space("alpha"), &group("writers"))
            .await
            .expect("created")
    );
    let write_only = Access {
        read: false,
        write: true,
    };
    for change in [
        groups
            .set_grant(
                &space("alpha"),
                &group("writers"),
                &bucket,
                &space("alpha"),
                write_only,
            )
            .await
            .expect("granted"),
        groups
            .add_member(&space("alpha"), &group("writers"), &user, &space("alpha"))
            .await
            .expect("added"),
    ] {
        assert_eq!(change, MemberChange::Done);
    }
    let write = call_as(
        &state,
        ann.pair(),
        "PUT",
        "/shared/b.txt",
        vec![],
        vec![],
        b"x",
    )
    .await;
    assert_eq!(write.status, 200, "{}", write.body);
    assert_eq!(
        read(&state, &ann).await.0,
        200,
        "reading still comes from the other group"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn taking_a_member_out_ends_the_access_within_the_window() {
    let _why = IGNORED;
    let (state, now) = state_with_clock().await;
    let (ann, _) = readers(state.storage(), &state).await;
    assert_eq!(read(&state, &ann).await.0, 200);
    let user = tessari_s3_core::authz::UserName::new("ann").expect("a user name");
    assert!(
        state
            .storage()
            .groups()
            .remove_member(&space("alpha"), &group("readers"), &user)
            .await
            .expect("removed")
    );
    assert_eq!(
        read(&state, &ann).await.0,
        200,
        "the cached resolution still answers"
    );
    now.fetch_add(4, Ordering::AcqRel);
    assert_eq!(read(&state, &ann).await.0, 200, "4 s after the resolution");
    now.fetch_add(1, Ordering::AcqRel);
    assert_eq!(
        read(&state, &ann).await,
        (403, Some("AccessDenied".to_owned())),
        "5 s after the resolution the membership is gone"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn removing_the_grant_or_the_group_ends_the_access_and_takes_its_records() {
    let _why = IGNORED;
    let (state, now) = state_with_clock().await;
    let (ann, _) = readers(state.storage(), &state).await;
    let groups = state.storage().groups();
    let bucket = BucketName::new("shared").expect("a bucket name");
    assert!(
        groups
            .remove_grant(&space("alpha"), &group("readers"), &bucket)
            .await
            .expect("removed")
    );
    now.fetch_add(5, Ordering::AcqRel);
    assert_eq!(read(&state, &ann).await.0, 403, "the grant is gone");
    assert_eq!(
        groups
            .set_grant(
                &space("alpha"),
                &group("readers"),
                &bucket,
                &space("alpha"),
                READ
            )
            .await
            .expect("granted"),
        MemberChange::Done
    );
    now.fetch_add(5, Ordering::AcqRel);
    assert_eq!(read(&state, &ann).await.0, 200, "granted again");
    assert!(
        groups
            .delete(&space("alpha"), &group("readers"))
            .await
            .expect("deleted")
    );
    now.fetch_add(5, Ordering::AcqRel);
    assert_eq!(read(&state, &ann).await.0, 403, "the group is gone");
    // Its records went with it: a group of the same name starts empty.
    assert!(
        groups
            .create(&space("alpha"), &group("readers"))
            .await
            .expect("created")
    );
    assert!(
        groups
            .members(&space("alpha"), &group("readers"))
            .await
            .expect("members")
            .is_empty()
    );
    assert!(
        groups
            .grants(&space("alpha"), &group("readers"))
            .await
            .expect("grants")
            .is_empty()
    );
    now.fetch_add(5, Ordering::AcqRel);
    assert_eq!(
        read(&state, &ann).await.0,
        403,
        "a recreated group grants nothing"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_group_never_reaches_across_spaces_and_a_missing_group_takes_nothing() {
    let _why = IGNORED;
    let (state, _) = state_with_clock().await;
    let storage = state.storage();
    let _carl = user_with_key(storage, "carl", "beta", Role::Member, false).await;
    let groups = storage.groups();
    assert!(
        groups
            .create(&space("alpha"), &group("readers"))
            .await
            .expect("created")
    );
    let carl = tessari_s3_core::authz::UserName::new("carl").expect("a user name");
    assert_eq!(
        groups
            .add_member(&space("alpha"), &group("readers"), &carl, &space("beta"))
            .await
            .expect("answered"),
        MemberChange::OtherSpace
    );
    let elsewhere = BucketName::new("elsewhere").expect("a bucket name");
    assert_eq!(
        groups
            .set_grant(
                &space("alpha"),
                &group("readers"),
                &elsewhere,
                &space("beta"),
                READ
            )
            .await
            .expect("answered"),
        MemberChange::OtherSpace
    );
    assert_eq!(
        groups
            .add_member(&space("alpha"), &group("nobody"), &carl, &space("alpha"))
            .await
            .expect("answered"),
        MemberChange::NoSuchGroup
    );
    assert!(
        groups
            .members(&space("alpha"), &group("nobody"))
            .await
            .expect("members")
            .is_empty(),
        "nothing was written for a missing group"
    );
}
