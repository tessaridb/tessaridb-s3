//! Users, their access keys and their grants, against a real node: an issued key resolves to its user and secret
//! until it is disabled or revoked or its user is disabled, and a principal carries the user's space, role and
//! grants. How the secret is stored is `users_sealed`.

use tessari_s3_core::authz::{Access, Role, SpaceName, UserName, Visible};
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_storage::Storage;
use tessari_s3_storage::spaces::SpaceCreated;
use tessari_s3_storage::users::{NewUser, User, UserCreated};
use tessari_s3_types::BucketName;

use crate::{IGNORED, live_config, prepared, test_node};

fn space(text: &str) -> SpaceName {
    SpaceName::new(text).expect("a space name")
}

pub(crate) fn user(text: &str) -> UserName {
    UserName::new(text).expect("a user name")
}

fn bucket(text: &str) -> BucketName {
    BucketName::new(text).expect("a bucket name")
}

fn member(name: &str, in_space: &str) -> NewUser {
    NewUser {
        name: user(name),
        space: space(in_space),
        role: Role::Member,
        create_buckets: false,
        operator: false,
        cluster_viewer: false,
    }
}

fn names(listed: Vec<User>) -> Vec<String> {
    listed
        .into_iter()
        .map(|user| user.name.as_str().to_owned())
        .collect()
}

/// Storage over a fresh namespace with the space `alpha` and its member `ann`, and a pool into the same namespace.
pub(crate) async fn with_ann() -> (Storage, MetaPool) {
    let config = live_config(test_node(), None, None);
    let storage = prepared(&config).await;
    assert!(matches!(
        storage
            .spaces()
            .create(&space("alpha"))
            .await
            .expect("space"),
        SpaceCreated::Created(_)
    ));
    assert!(matches!(
        storage
            .users()
            .create(&member("ann", "alpha"))
            .await
            .expect("user"),
        UserCreated::Created(_)
    ));
    (storage, MetaPool::new(config.meta).expect("pool"))
}

async fn resolves(storage: &Storage, id: &str) -> bool {
    storage
        .users()
        .resolve(id)
        .await
        .expect("answered")
        .is_some()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_issued_key_resolves_to_its_user_and_secret() {
    let _why = IGNORED;
    let (storage, _) = with_ann().await;
    let users = storage.users();
    let issued = users
        .issue_key(&user("ann"))
        .await
        .expect("issued")
        .expect("ann exists");
    assert!(issued.access_key_id.starts_with("TS"));
    assert_eq!(issued.access_key_id.len(), 20);
    assert_eq!(issued.secret.expose().len(), 40);

    let resolved = users
        .resolve(&issued.access_key_id)
        .await
        .expect("resolved")
        .expect("a live key");
    assert_eq!(resolved.user.name, user("ann"));
    assert_eq!(resolved.user.space, space("alpha"));
    assert_eq!(resolved.secret.expose(), issued.secret.expose());
    assert!(!resolves(&storage, "TSNOSUCHKEY000000000").await);
    assert!(
        users
            .issue_key(&user("nobody"))
            .await
            .expect("answered")
            .is_none()
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_disabled_or_revoked_key_or_a_disabled_user_resolves_to_nothing() {
    let _why = IGNORED;
    let (storage, _) = with_ann().await;
    let users = storage.users();
    let ann = user("ann");
    let first = users.issue_key(&ann).await.expect("issued").expect("ann");
    let second = users.issue_key(&ann).await.expect("issued").expect("ann");

    assert!(
        users
            .set_key_disabled(&first.access_key_id, true)
            .await
            .expect("set")
    );
    assert!(
        !resolves(&storage, &first.access_key_id).await,
        "a disabled key"
    );
    assert!(
        resolves(&storage, &second.access_key_id).await,
        "the other key still works"
    );

    assert!(users.set_disabled(&ann, true).await.expect("set"));
    assert!(
        !resolves(&storage, &second.access_key_id).await,
        "a disabled user's key"
    );
    assert!(users.principal(&ann).await.expect("answered").is_none());
    assert!(users.set_disabled(&ann, false).await.expect("set"));
    assert!(
        resolves(&storage, &second.access_key_id).await,
        "enabled again"
    );

    assert!(
        users
            .revoke_key(&second.access_key_id)
            .await
            .expect("revoked")
    );
    assert!(
        !resolves(&storage, &second.access_key_id).await,
        "a revoked key"
    );
    assert!(
        !users
            .revoke_key(&second.access_key_id)
            .await
            .expect("answered")
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_principal_carries_the_space_role_and_grants_of_its_user() {
    let _why = IGNORED;
    let (storage, _) = with_ann().await;
    let users = storage.users();
    let ann = user("ann");
    let read = Access {
        read: true,
        write: false,
    };
    let both = Access {
        read: true,
        write: true,
    };
    assert!(
        users
            .set_grant(&ann, &bucket("photos"), read)
            .await
            .expect("granted")
    );
    assert!(
        users
            .set_grant(&ann, &bucket("videos"), read)
            .await
            .expect("granted")
    );
    assert!(
        users
            .set_grant(&ann, &bucket("videos"), both)
            .await
            .expect("replaced")
    );
    assert!(
        users
            .remove_grant(&ann, &bucket("videos"))
            .await
            .expect("removed")
    );
    assert!(
        !users
            .remove_grant(&ann, &bucket("videos"))
            .await
            .expect("answered")
    );
    assert!(
        !users
            .set_grant(&user("nobody"), &bucket("photos"), read)
            .await
            .expect("answered")
    );

    let principal = users.principal(&ann).await.expect("answered").expect("ann");
    assert_eq!(principal.name, "ann");
    assert_eq!(principal.space, space("alpha"));
    assert_eq!(principal.role, Role::Member);
    assert_eq!(
        principal.grants.into_iter().collect::<Vec<_>>(),
        vec![("photos".to_owned(), read)]
    );

    let admin = NewUser {
        role: Role::SpaceAdmin,
        ..member("bob", "alpha")
    };
    assert!(matches!(
        users.create(&admin).await.expect("bob"),
        UserCreated::Created(_)
    ));
    assert!(matches!(
        users.create(&admin).await.expect("again"),
        UserCreated::Exists
    ));
    assert!(matches!(
        users
            .create(&member("cid", "gamma"))
            .await
            .expect("answered"),
        UserCreated::NoSuchSpace
    ));
    assert_eq!(
        names(
            users
                .list(&Visible::Space(space("alpha")))
                .await
                .expect("listed")
        ),
        vec!["ann", "bob"]
    );
    assert!(
        names(
            users
                .list(&Visible::Space(space("beta")))
                .await
                .expect("listed")
        )
        .is_empty()
    );
    let bob = users
        .principal(&user("bob"))
        .await
        .expect("answered")
        .expect("bob");
    assert_eq!(bob.role, Role::SpaceAdmin);
}
