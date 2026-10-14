//! Console sign-in through an identity provider, its records against a real node: an in-flight login is taken exactly
//! once — also when several callbacks race for it — and an identity resolves to the one user it is linked to, moves
//! when that user is linked again, and is never silently taken from another user.

use tessari_s3_storage::identities::{Linked, PendingLogin};
use tessari_s3_types::SecretKey;

use crate::IGNORED;
use crate::users::{user, with_ann};

const ISSUER: &str = "https://id.example.test";
const LIFETIME: i64 = 600;

fn pending(state: &str) -> PendingLogin {
    PendingLogin {
        state: state.to_owned(),
        nonce: SecretKey::new(format!("nonce-of-{state}")),
        verifier: SecretKey::new(format!("verifier-of-{state}")),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_in_flight_login_is_taken_once_and_only_by_its_state() {
    let _why = IGNORED;
    let (storage, _) = with_ann().await;
    let identities = storage.identities();
    identities
        .begin_login(&pending("state-one-0123456789"), LIFETIME)
        .await
        .expect("written");
    assert!(
        identities
            .take_login("state-two-0123456789")
            .await
            .expect("answered")
            .is_none(),
        "another state takes nothing"
    );
    let taken = identities
        .take_login("state-one-0123456789")
        .await
        .expect("answered")
        .expect("the login");
    assert_eq!(taken.state, "state-one-0123456789");
    assert_eq!(taken.nonce.expose(), "nonce-of-state-one-0123456789");
    assert_eq!(taken.verifier.expose(), "verifier-of-state-one-0123456789");
    assert!(
        identities
            .take_login("state-one-0123456789")
            .await
            .expect("answered")
            .is_none(),
        "a state is good for one callback"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn racing_callbacks_take_one_login_exactly_once() {
    let _why = IGNORED;
    let (storage, _) = with_ann().await;
    let identities = storage.identities().clone();
    for round in 0..5 {
        let state = format!("state-race-{round}-0123456789");
        identities
            .begin_login(&pending(&state), LIFETIME)
            .await
            .expect("written");
        let takers: Vec<_> = (0..8)
            .map(|_| {
                let identities = identities.clone();
                let state = state.clone();
                tokio::spawn(async move { identities.take_login(&state).await })
            })
            .collect();
        let mut won = 0_u32;
        for taker in takers {
            if taker.await.expect("joined").expect("answered").is_some() {
                won += 1;
            }
        }
        assert_eq!(won, 1, "round {round}: exactly one callback took the login");
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_identity_resolves_to_its_one_user_and_moves_only_with_that_user() {
    let _why = IGNORED;
    let (storage, _) = with_ann().await;
    let identities = storage.identities();
    let resolve = |subject: &'static str| async move {
        identities
            .user_for(ISSUER, subject)
            .await
            .expect("answered")
            .map(|name| name.as_str().to_owned())
    };
    assert_eq!(resolve("subject-1").await, None);
    assert_eq!(
        identities
            .link(&user("ann"), ISSUER, "subject-1")
            .await
            .expect("linked"),
        Linked::Linked
    );
    assert_eq!(resolve("subject-1").await.as_deref(), Some("ann"));
    // The same subject at another issuer is another identity.
    assert_eq!(
        identities
            .user_for("https://other.example.test", "subject-1")
            .await
            .expect("answered"),
        None
    );
    // Linking ann again replaces her link: the old subject resolves to nobody.
    identities
        .link(&user("ann"), ISSUER, "subject-2")
        .await
        .expect("linked");
    assert_eq!(resolve("subject-1").await, None);
    assert_eq!(resolve("subject-2").await.as_deref(), Some("ann"));
    // A subject linked to ann is not handed to another user.
    assert_eq!(
        identities
            .link(&user("bob"), ISSUER, "subject-2")
            .await
            .expect("answered"),
        Linked::TakenByAnother
    );
    assert_eq!(resolve("subject-2").await.as_deref(), Some("ann"));
    assert!(identities.unlink(&user("ann")).await.expect("unlinked"));
    assert_eq!(resolve("subject-2").await, None);
    assert!(!identities.unlink(&user("ann")).await.expect("answered"));
}
