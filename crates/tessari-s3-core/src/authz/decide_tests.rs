use std::collections::BTreeMap;

use super::{
    Access, Action, BucketResource, Decision, Denied, Principal, Role, SpaceName, UserName,
    UserPrincipal, UserResource, authorize,
};

fn space(name: &str) -> SpaceName {
    SpaceName::new(name).expect("a valid space name")
}

fn user(name: &str, in_space: &str, role: Role) -> UserPrincipal {
    UserPrincipal {
        name: name.to_owned(),
        space: space(in_space),
        role,
        create_buckets: false,
        operator: false,
        cluster_viewer: false,
        grants: BTreeMap::new(),
    }
}

fn bucket(name: &str, in_space: &str, creator: Option<&str>) -> BucketResource {
    BucketResource {
        name: name.to_owned(),
        space: space(in_space),
        creator: creator.map(str::to_owned),
    }
}

fn decide(principal: UserPrincipal, action: &Action) -> Decision {
    authorize(&Principal::User(principal), action)
}

const DENY_GRANT: Decision = Decision::Deny(Denied::NoGrant);
const DENY_ROLE: Decision = Decision::Deny(Denied::Role);
const DENY_SPACE: Decision = Decision::Deny(Denied::OtherSpace);

#[test]
fn space_names_follow_the_node_name_rule() {
    assert!(SpaceName::new("team-a1").is_some());
    for bad in ["", "-a", "a-", "Team", "a_b", &"a".repeat(64)] {
        assert!(SpaceName::new(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn user_names_follow_the_same_rule() {
    assert_eq!(
        UserName::new("ann-2").map(|name| name.as_str().to_owned()),
        Some("ann-2".to_owned())
    );
    for bad in ["", "-a", "a-", "Ann", "a/b", &"a".repeat(64)] {
        assert!(UserName::new(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn root_and_operators_may_do_anything() {
    let other = bucket("photos", "beta", None);
    let actions = [
        Action::Operate,
        Action::ViewCluster,
        Action::ManageSpace(space("beta")),
        Action::DeleteBucket(other.clone()),
        Action::WriteObject(other),
    ];
    let mut operator = user("ops", "alpha", Role::Member);
    operator.operator = true;
    for action in &actions {
        assert_eq!(authorize(&Principal::Root, action), Decision::Allow);
        assert_eq!(decide(operator.clone(), action), Decision::Allow);
    }
}

#[test]
fn the_cluster_is_seen_only_by_its_viewers() {
    let mut viewer = user("watcher", "alpha", Role::Member);
    assert_eq!(decide(viewer.clone(), &Action::ViewCluster), DENY_ROLE);
    assert_eq!(
        decide(
            user("admin", "alpha", Role::SpaceAdmin),
            &Action::ViewCluster
        ),
        DENY_ROLE
    );
    viewer.cluster_viewer = true;
    assert_eq!(
        decide(viewer.clone(), &Action::ViewCluster),
        Decision::Allow
    );
    assert_eq!(decide(viewer, &Action::Operate), DENY_ROLE);
}

#[test]
fn a_space_is_administered_only_by_its_own_admin() {
    let admin = user("admin", "alpha", Role::SpaceAdmin);
    assert_eq!(
        decide(admin.clone(), &Action::ManageSpace(space("alpha"))),
        Decision::Allow
    );
    assert_eq!(
        decide(admin, &Action::ManageSpace(space("beta"))),
        DENY_SPACE
    );
    assert_eq!(
        decide(
            user("ann", "alpha", Role::Member),
            &Action::ManageSpace(space("alpha"))
        ),
        DENY_ROLE
    );
}

#[test]
fn buckets_are_created_in_ones_own_space_when_allowed() {
    let mut member = user("ann", "alpha", Role::Member);
    let here = Action::CreateBucket(space("alpha"));
    assert_eq!(decide(member.clone(), &here), DENY_ROLE);
    member.create_buckets = true;
    assert_eq!(decide(member.clone(), &here), Decision::Allow);
    assert_eq!(
        decide(member, &Action::CreateBucket(space("beta"))),
        DENY_SPACE
    );
    assert_eq!(
        decide(user("admin", "alpha", Role::SpaceAdmin), &here),
        Decision::Allow
    );
}

#[test]
fn every_user_may_ask_for_their_bucket_list() {
    assert_eq!(
        decide(user("ann", "alpha", Role::Member), &Action::ListBuckets),
        Decision::Allow
    );
}

#[test]
fn a_member_reaches_a_bucket_only_through_a_grant_or_by_creating_it() {
    let photos = bucket("photos", "alpha", Some("bob"));
    let member = user("ann", "alpha", Role::Member);
    for action in [
        Action::ReadBucket(photos.clone()),
        Action::ReadObject(photos.clone()),
        Action::WriteObject(photos.clone()),
        Action::DeleteBucket(photos.clone()),
    ] {
        assert_eq!(decide(member.clone(), &action), DENY_GRANT, "{action:?}");
    }
    let creator = user("bob", "alpha", Role::Member);
    for action in [
        Action::ReadObject(photos.clone()),
        Action::WriteObject(photos.clone()),
        Action::DeleteBucket(photos),
    ] {
        assert_eq!(
            decide(creator.clone(), &action),
            Decision::Allow,
            "{action:?}"
        );
    }
}

#[test]
fn a_grant_allows_exactly_what_it_names() {
    let photos = bucket("photos", "alpha", None);
    let mut reader = user("ann", "alpha", Role::Member);
    reader.grants.insert(
        "photos".to_owned(),
        Access {
            read: true,
            write: false,
        },
    );
    assert_eq!(
        decide(reader.clone(), &Action::ReadBucket(photos.clone())),
        Decision::Allow
    );
    assert_eq!(
        decide(reader.clone(), &Action::ReadObject(photos.clone())),
        Decision::Allow
    );
    assert_eq!(
        decide(reader.clone(), &Action::WriteObject(photos.clone())),
        DENY_GRANT
    );
    assert_eq!(
        decide(reader, &Action::ReadObject(bucket("videos", "alpha", None))),
        DENY_GRANT
    );

    let mut writer = user("cid", "alpha", Role::Member);
    writer.grants.insert(
        "photos".to_owned(),
        Access {
            read: false,
            write: true,
        },
    );
    assert_eq!(
        decide(writer.clone(), &Action::WriteObject(photos.clone())),
        Decision::Allow
    );
    assert_eq!(
        decide(writer.clone(), &Action::ReadObject(photos.clone())),
        DENY_GRANT
    );
    assert_eq!(
        decide(writer, &Action::DeleteBucket(photos)),
        DENY_GRANT,
        "a write grant does not delete the bucket"
    );
}

#[test]
fn a_space_admin_reaches_every_bucket_of_the_space() {
    let photos = bucket("photos", "alpha", Some("bob"));
    let admin = user("admin", "alpha", Role::SpaceAdmin);
    for action in [
        Action::ReadObject(photos.clone()),
        Action::WriteObject(photos.clone()),
        Action::DeleteBucket(photos),
    ] {
        assert_eq!(
            decide(admin.clone(), &action),
            Decision::Allow,
            "{action:?}"
        );
    }
}

#[test]
fn nothing_crosses_from_one_space_into_another() {
    let theirs = bucket("photos", "beta", Some("ann"));
    let mut admin = user("ann", "alpha", Role::SpaceAdmin);
    admin.create_buckets = true;
    admin.cluster_viewer = true;
    admin.grants.insert(
        "photos".to_owned(),
        Access {
            read: true,
            write: true,
        },
    );
    for action in [
        Action::ReadBucket(theirs.clone()),
        Action::ReadObject(theirs.clone()),
        Action::WriteObject(theirs.clone()),
        Action::DeleteBucket(theirs),
        Action::CreateBucket(space("beta")),
        Action::ManageSpace(space("beta")),
    ] {
        assert_eq!(decide(admin.clone(), &action), DENY_SPACE, "{action:?}");
    }
}

#[test]
fn a_listing_sees_one_space_unless_the_caller_operates() {
    use super::Visible;
    assert_eq!(Principal::Root.visible(), Visible::All);
    let admin = user("ann", "alpha", Role::SpaceAdmin);
    assert_eq!(
        Principal::User(admin.clone()).visible(),
        Visible::Space(space("alpha"))
    );
    let mut member = user("bob", "alpha", Role::Member);
    member.grants.insert("shared".to_owned(), Access::default());
    member.grants.insert(
        "photos".to_owned(),
        Access {
            read: false,
            write: true,
        },
    );
    assert_eq!(
        Principal::User(member).visible(),
        Visible::Reachable {
            space: space("alpha"),
            user: "bob".to_owned(),
            granted: vec!["photos".to_owned()],
        },
        "a member lists what it created or may read or write — a grant that allows nothing reaches nothing"
    );
    let mut operator = admin;
    operator.operator = true;
    assert_eq!(Principal::User(operator).visible(), Visible::All);
}

fn target(name: &str, in_space: &str, role: Role) -> UserResource {
    UserResource {
        name: name.to_owned(),
        space: space(in_space),
        role,
        operator: false,
        cluster_viewer: false,
    }
}

#[test]
fn a_space_admin_manages_only_the_plain_members_of_its_own_space() {
    let admin = user("boss", "alpha", Role::SpaceAdmin);
    let member = target("ann", "alpha", Role::Member);
    assert_eq!(
        decide(admin.clone(), &Action::ManageUser(member.clone())),
        Decision::Allow
    );
    assert_eq!(
        decide(
            admin.clone(),
            &Action::ManageUser(target("bob", "beta", Role::Member))
        ),
        DENY_SPACE
    );
    let mut viewer = member.clone();
    viewer.cluster_viewer = true;
    let mut operator = member;
    operator.operator = true;
    for above in [target("peer", "alpha", Role::SpaceAdmin), viewer, operator] {
        assert_eq!(
            decide(admin.clone(), &Action::ManageUser(above.clone())),
            DENY_ROLE,
            "a space admin cannot create or manage authority equal to or above its own: {above:?}"
        );
        assert_eq!(
            authorize(&Principal::Root, &Action::ManageUser(above)),
            Decision::Allow
        );
    }
    assert_eq!(
        decide(
            user("cid", "alpha", Role::Member),
            &Action::ManageUser(target("ann", "alpha", Role::Member))
        ),
        DENY_ROLE
    );
}

#[test]
fn nobody_manages_their_own_user() {
    let admin = user("boss", "alpha", Role::SpaceAdmin);
    let mut operator = user("ops", "alpha", Role::Member);
    operator.operator = true;
    assert_eq!(
        decide(
            admin,
            &Action::ManageUser(target("boss", "alpha", Role::Member))
        ),
        Decision::Deny(Denied::SelfManagement)
    );
    assert_eq!(
        decide(
            operator.clone(),
            &Action::ManageUser(target("ops", "alpha", Role::Member))
        ),
        Decision::Deny(Denied::SelfManagement),
        "the store-wide role does not lift it"
    );
    assert_eq!(
        decide(
            operator,
            &Action::ManageUser(target("ann", "beta", Role::SpaceAdmin))
        ),
        Decision::Allow
    );
}
