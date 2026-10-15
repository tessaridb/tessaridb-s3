//! Console sign-in through an identity provider, end to end against a real node and the test provider: a person
//! whose identity is linked to a user signs in as that user, and every other return — an unlinked identity, no
//! second factor, a state never issued or already spent, a code bound to another sign-in, a disabled user — is the
//! same refusal with no session. The session names the person, and ends at the next request once the user is
//! disabled. Linking an identity is an operator's recorded act, and an identity is never taken from another user.

use serde_json::Value;
use tessari_s3_api::{ApiState, ConsoleState};
use tessari_s3_core::authz::{Role, SpaceName};
use tessari_s3_infrastructure::{KeySignIn, OidcSettings};
use tessari_s3_storage::Storage;
use tessari_s3_storage::users::NewUser;
use tessari_s3_types::SecretKey;

use crate::console::{Seen, send, sign_in};
use crate::oidc_provider::{CLIENT_ID, CLIENT_SECRET, Provider, REDIRECT, SUBJECT, Shape};
use crate::users::user;
use crate::{IGNORED, live_config, prepared, scratch_dir, test_node};

/// A console over a fresh store whose identity provider is `provider` (announcing a root sign-in to `webhook`, when
/// given), with the space `alpha` and its members `ann` and `bob`.
pub(crate) async fn console_with(
    provider: &Provider,
    webhook: Option<String>,
) -> (ConsoleState, Storage) {
    let (mut consoles, storage) = consoles_with(provider, webhook, &[KeySignIn::All]).await;
    (consoles.remove(0), storage)
}

/// One console per entry of `modes`, all over one fresh store and one root credential, so a session issued by one
/// is presented to another as it would be after a restart with the other setting.
pub(crate) async fn consoles_with(
    provider: &Provider,
    webhook: Option<String>,
    modes: &[KeySignIn],
) -> (Vec<ConsoleState>, Storage) {
    let mut config = live_config(test_node(), None, &scratch_dir());
    let console = config.console.as_mut().expect("a console");
    // These tests start more sign-ins a minute, from one address, than the default limit lets through.
    console.sign_ins_per_minute = 100;
    console.oidc = Some(OidcSettings {
        issuer: provider.issuer.clone(),
        client_id: CLIENT_ID.to_owned(),
        client_secret: SecretKey::new(CLIENT_SECRET.to_owned()),
        redirect_uri: REDIRECT.to_owned(),
        mfa_amr: vec!["mfa".to_owned()],
        break_glass_webhook: webhook,
        key_sign_in: KeySignIn::All,
    });
    let storage = prepared(&config).await;
    let alpha = SpaceName::new("alpha").expect("a space name");
    storage.spaces().create(&alpha).await.expect("space");
    for name in ["ann", "bob"] {
        storage
            .users()
            .create(&NewUser {
                name: user(name),
                space: alpha.clone(),
                role: Role::Member,
                create_buckets: false,
                operator: false,
                cluster_viewer: false,
            })
            .await
            .expect("user");
    }
    let consoles = modes
        .iter()
        .map(|mode| {
            let mut config = config.clone();
            if let Some(oidc) = config
                .console
                .as_mut()
                .and_then(|console| console.oidc.as_mut())
            {
                oidc.key_sign_in = *mode;
            }
            ConsoleState::new(&config, ApiState::system_clock(), storage.clone())
                .expect("built")
                .expect("a console is configured")
        })
        .collect();
    (consoles, storage)
}

/// Starts a sign-in: the provider's authorization URL and the state in it.
async fn start(console: &ConsoleState) -> (String, String) {
    let answer = send(console, "GET", "/api/v1/auth/oidc/start", None, None).await;
    assert_eq!(answer.status, 303);
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    let url = answer.header("location").expect("a location").to_owned();
    let state = openidconnect::url::Url::parse(&url)
        .expect("a URL")
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .expect("a state");
    (url, state)
}

async fn callback(console: &ConsoleState, query: &str) -> Seen {
    send(
        console,
        "GET",
        &format!("/api/v1/auth/oidc/callback?{query}"),
        None,
        None,
    )
    .await
}

/// A person's full sign-in, answering the callback's response.
async fn sign_in_through(
    provider: &Provider,
    console: &ConsoleState,
    code: &str,
    shape: Shape,
) -> Seen {
    let (url, state) = start(console).await;
    provider.grant(&url, code, shape);
    callback(console, &format!("code={code}&state={state}")).await
}

fn refused(seen: &Seen, case: &str) {
    assert_eq!(seen.status, 401, "{case}");
    assert_eq!(seen.code().as_deref(), Some("sign_in_refused"), "{case}");
    assert!(seen.cookie().is_none(), "{case}: no session");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_linked_person_signs_in_as_the_user_once_per_state() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (console, storage) = console_with(&provider, None).await;
    storage
        .identities()
        .link(&user("ann"), &provider.issuer, SUBJECT)
        .await
        .expect("linked");
    let methods = send(&console, "GET", "/api/v1/auth/methods", None, None).await;
    assert_eq!(methods.json()["provider"], Value::Bool(true));
    let (url, state) = start(&console).await;
    provider.grant(&url, "code-ann", Shape::Good);
    let signed = callback(&console, &format!("code=code-ann&state={state}")).await;
    assert_eq!(signed.status, 303);
    assert_eq!(signed.header("location"), Some("/"));
    let token = signed.cookie().expect("a session");
    let session = send(&console, "GET", "/api/v1/session", Some(&token), None).await;
    assert_eq!(session.status, 200);
    assert_eq!(session.json()["access_key_id"], Value::from("user:ann"));
    // The same return again: its state is spent.
    refused(
        &callback(&console, &format!("code=code-ann&state={state}")).await,
        "a replayed state",
    );
    // Once ann is disabled, the session she holds ends at the next request.
    storage
        .users()
        .set_disabled(&user("ann"), true)
        .await
        .expect("disabled");
    let after = send(&console, "GET", "/api/v1/session", Some(&token), None).await;
    assert_eq!(after.status, 401);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_other_return_is_the_same_refusal_and_no_session() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (console, storage) = console_with(&provider, None).await;
    // Nobody is linked yet: a correct sign-in of an unlinked identity.
    refused(
        &sign_in_through(&provider, &console, "code-unlinked", Shape::Good).await,
        "an unlinked identity",
    );
    storage
        .identities()
        .link(&user("ann"), &provider.issuer, SUBJECT)
        .await
        .expect("linked");
    refused(
        &sign_in_through(&provider, &console, "code-pwd", Shape::NoSecondFactor).await,
        "no second factor",
    );
    refused(
        &sign_in_through(&provider, &console, "code-nonce", Shape::WrongNonce).await,
        "another nonce",
    );
    refused(
        &callback(&console, "code=anything&state=never-issued-0123456789").await,
        "a state never issued",
    );
    let (_, state) = start(&console).await;
    refused(
        &callback(&console, &format!("state={state}")).await,
        "no code",
    );
    // A code granted to one sign-in, returned with another sign-in's state: the verifiers differ.
    let (first, _) = start(&console).await;
    let (_, second) = start(&console).await;
    provider.grant(&first, "code-crossed", Shape::Good);
    refused(
        &callback(&console, &format!("code=code-crossed&state={second}")).await,
        "a code bound to another sign-in",
    );
    // The provider's own error return.
    let (_, state) = start(&console).await;
    refused(
        &callback(&console, &format!("error=access_denied&state={state}")).await,
        "the provider's error",
    );
    storage
        .users()
        .set_disabled(&user("ann"), true)
        .await
        .expect("disabled");
    refused(
        &sign_in_through(&provider, &console, "code-disabled", Shape::Good).await,
        "a disabled user",
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_operator_links_and_unlinks_an_identity_on_the_record() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (console, _) = console_with(&provider, None).await;
    let root = sign_in(&console).await;
    let link = |user: &'static str, subject: &'static str| {
        let console = console.clone();
        let root = root.clone();
        async move {
            send(
                &console,
                "PUT",
                &format!("/api/v1/users/{user}/identity"),
                Some(&root),
                Some(&format!(
                    "{{\"subject\":\"{subject}\",\"reason\":\"ticket 42\"}}"
                )),
            )
            .await
        }
    };
    assert_eq!(link("ann", SUBJECT).await.status, 204);
    let signed = sign_in_through(&provider, &console, "code-linked", Shape::Good).await;
    assert_eq!(signed.status, 303, "the linked identity signs in");
    let taken = link("bob", SUBJECT).await;
    assert_eq!(taken.status, 409);
    assert_eq!(taken.code().as_deref(), Some("identity_taken"));
    let unlink = || {
        let console = console.clone();
        let root = root.clone();
        async move {
            send(
                &console,
                "DELETE",
                "/api/v1/users/ann/identity",
                Some(&root),
                Some("{\"reason\":\"ticket 43\"}"),
            )
            .await
        }
    };
    assert_eq!(unlink().await.status, 204);
    assert_eq!(unlink().await.code().as_deref(), Some("no_identity"));
    refused(
        &sign_in_through(&provider, &console, "code-unlinked", Shape::Good).await,
        "an unlinked identity",
    );
    let actions = send(&console, "GET", "/api/v1/actions", Some(&root), None)
        .await
        .json();
    let operations: Vec<&str> = actions["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter_map(|action| action["operation"].as_str())
        .collect();
    assert!(operations.contains(&"link_identity"), "{operations:?}");
    assert!(operations.contains(&"unlink_identity"), "{operations:?}");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn without_an_identity_provider_there_is_no_such_route() {
    let _why = IGNORED;
    let config = live_config(test_node(), None, &scratch_dir());
    let storage = prepared(&config).await;
    let console = ConsoleState::new(&config, ApiState::system_clock(), storage)
        .expect("built")
        .expect("a console is configured");
    let answer = send(&console, "GET", "/api/v1/auth/oidc/start", None, None).await;
    assert_eq!(answer.status, 404);
    // The sign-in page asks, without a session, whether to offer the provider.
    let methods = send(&console, "GET", "/api/v1/auth/methods", None, None).await;
    assert_eq!(methods.status, 200);
    assert_eq!(methods.json()["provider"], Value::Bool(false));
    assert_eq!(methods.json()["keys"], Value::from("all"));
}
