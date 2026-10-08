//! Who may do what over S3, against a real node: a user's key signs with the user's own secret; it creates buckets in
//! its own space when allowed and lists only that space, and it reaches another user's bucket only through a grant.
//! Crossing spaces and copying are `access_crossing`.

use tessari_s3_core::authz::{Access, Role, SpaceName, UserName};
use tessari_s3_storage::Storage;
use tessari_s3_storage::spaces::SpaceCreated;
use tessari_s3_storage::users::{NewUser, UserCreated};
use tessari_s3_types::BucketName;

use crate::{IGNORED, call, call_as, fresh};

/// A user's credential, as a client holds it.
pub(crate) struct Key {
    pub(crate) id: String,
    pub(crate) secret: String,
}

impl Key {
    pub(crate) fn pair(&self) -> (&str, &str) {
        (&self.id, &self.secret)
    }
}

pub(crate) fn space(text: &str) -> SpaceName {
    SpaceName::new(text).expect("a space name")
}

/// Ensures `in_space` exists, creates `name` there with `role` and `create_buckets`, and issues it a key.
pub(crate) async fn user_with_key(
    storage: &Storage,
    name: &str,
    in_space: &str,
    role: Role,
    create_buckets: bool,
) -> Key {
    let in_space = space(in_space);
    let created = storage.spaces().create(&in_space).await.expect("space");
    assert!(matches!(
        created,
        SpaceCreated::Created(_) | SpaceCreated::Exists
    ));
    let user = UserName::new(name).expect("a user name");
    let new = NewUser {
        name: user.clone(),
        space: in_space,
        role,
        create_buckets,
        operator: false,
        cluster_viewer: false,
    };
    assert!(matches!(
        storage.users().create(&new).await.expect("user"),
        UserCreated::Created(_)
    ));
    let issued = storage
        .users()
        .issue_key(&user)
        .await
        .expect("issued")
        .expect("the user exists");
    Key {
        id: issued.access_key_id,
        secret: issued.secret.expose().to_owned(),
    }
}

pub(crate) async fn grant(storage: &Storage, user: &str, bucket: &str, access: Access) {
    let user = UserName::new(user).expect("a user name");
    let bucket = BucketName::new(bucket).expect("a bucket name");
    assert!(
        storage
            .users()
            .set_grant(&user, &bucket, access)
            .await
            .expect("granted")
    );
}

const READ: Access = Access {
    read: true,
    write: false,
};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_user_creates_buckets_in_its_own_space_and_lists_only_that_space() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    assert_eq!(
        call(&state, "PUT", "/rooted", vec![], b"").await.status,
        200
    );

    let created = call_as(
        &state,
        ann.pair(),
        "PUT",
        "/ann-photos",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    let bucket = storage
        .buckets()
        .get(&BucketName::new("ann-photos").expect("a name"))
        .await
        .expect("read")
        .expect("created");
    assert_eq!(bucket.space, space("alpha"));
    assert_eq!(bucket.creator.as_deref(), Some("ann"));

    let listed = call_as(&state, ann.pair(), "GET", "/", vec![], vec![], b"").await;
    assert_eq!(listed.status, 200);
    assert!(listed.body.contains("<Name>ann-photos</Name>"));
    assert!(
        !listed.body.contains("<Name>rooted</Name>"),
        "{}",
        listed.body
    );
    let root_listed = call(&state, "GET", "/", vec![], b"").await;
    assert!(root_listed.body.contains("<Name>rooted</Name>"));
    assert!(root_listed.body.contains("<Name>ann-photos</Name>"));

    let refused = call_as(&state, bob.pair(), "PUT", "/bobs-own", vec![], vec![], b"").await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (403, Some("AccessDenied"))
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_member_reaches_another_users_bucket_only_as_far_as_its_grant() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    let cid = user_with_key(storage, "cid", "alpha", Role::Member, false).await;
    let admin = user_with_key(storage, "boss", "alpha", Role::SpaceAdmin, false).await;
    assert_eq!(
        call_as(
            &state,
            ann.pair(),
            "PUT",
            "/ann-photos",
            vec![],
            vec![],
            b""
        )
        .await
        .status,
        200
    );
    let put = call_as(
        &state,
        ann.pair(),
        "PUT",
        "/ann-photos/a.txt",
        vec![],
        vec![],
        b"hello",
    )
    .await;
    assert_eq!(put.status, 200);
    grant(storage, "bob", "ann-photos", READ).await;

    let read = call_as(
        &state,
        bob.pair(),
        "GET",
        "/ann-photos/a.txt",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!((read.status, read.body.as_str()), (200, "hello"));
    let listed = call_as(
        &state,
        bob.pair(),
        "GET",
        "/ann-photos",
        vec![("list-type", Some("2"))],
        vec![],
        b"",
    )
    .await;
    assert_eq!(listed.status, 200);
    for (method, path) in [
        ("PUT", "/ann-photos/b.txt"),
        ("DELETE", "/ann-photos/a.txt"),
        ("DELETE", "/ann-photos"),
    ] {
        let refused = call_as(&state, bob.pair(), method, path, vec![], vec![], b"x").await;
        assert_eq!(
            (refused.status, refused.code.as_deref()),
            (403, Some("AccessDenied")),
            "{method} {path}"
        );
    }
    let ungranted = call_as(
        &state,
        cid.pair(),
        "GET",
        "/ann-photos/a.txt",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(
        (ungranted.status, ungranted.code.as_deref()),
        (403, Some("AccessDenied"))
    );
    let administered = call_as(
        &state,
        admin.pair(),
        "PUT",
        "/ann-photos/c.txt",
        vec![],
        vec![],
        b"c",
    )
    .await;
    assert_eq!(
        administered.status, 200,
        "a space admin reaches every bucket of its space"
    );
}
