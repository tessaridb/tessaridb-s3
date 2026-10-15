//! The console's identity-provider client against a provider that answers in the shape each test chooses: a correct
//! sign-in names its subject, and every way the return leg can be wrong is refused with its own reason — a token for
//! another nonce, client or issuer, an expired one, `alg: none`, an HMAC forged with the client secret, an `at_hash`
//! of another access token, no second factor, an `iss` response parameter naming another issuer, a code redeemed
//! with the wrong PKCE verifier or a second time. Keys rotate transparently, and an unknown key id refreshes the key
//! set at most once per interval.

use tessari_s3_infrastructure::OidcSettings;
use tessari_s3_infrastructure::oidc::{OidcClient, OidcError, Started};
use tessari_s3_types::SecretKey;

use crate::oidc_provider::{CLIENT_ID, CLIENT_SECRET, Provider, REDIRECT, SUBJECT, Shape};

fn settings(provider: &Provider) -> OidcSettings {
    OidcSettings {
        issuer: provider.issuer.clone(),
        client_id: CLIENT_ID.to_owned(),
        client_secret: SecretKey::new(CLIENT_SECRET.to_owned()),
        redirect_uri: REDIRECT.to_owned(),
        mfa_amr: vec!["mfa".to_owned()],
        break_glass_webhook: None,
        key_sign_in: tessari_s3_infrastructure::KeySignIn::All,
    }
}

fn client(provider: &Provider, refresh_min_secs: i64) -> OidcClient {
    OidcClient::new(&settings(provider), refresh_min_secs).expect("a client")
}

/// Begins a sign-in, has the provider grant `code` in `shape`, and finishes it.
async fn sign_in(
    provider: &Provider,
    client: &OidcClient,
    code: &str,
    shape: Shape,
) -> Result<String, OidcError> {
    let started = client.begin(None).await.expect("begun");
    provider.grant(&started.url, code, shape);
    finish(client, &started, code).await
}

async fn finish(client: &OidcClient, started: &Started, code: &str) -> Result<String, OidcError> {
    client
        .finish(code, None, &started.nonce, &started.verifier)
        .await
        .map(|identity| identity.subject)
}

#[tokio::test]
async fn the_authorization_request_carries_pkce_state_and_nonce() {
    let provider = Provider::start().await;
    let started = client(&provider, 60).begin(None).await.expect("begun");
    let url = openidconnect::url::Url::parse(&started.url).expect("a URL");
    assert!(
        started
            .url
            .starts_with(&format!("{}/authorize?", provider.issuer))
    );
    let pair = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    assert_eq!(pair("response_type").as_deref(), Some("code"));
    assert_eq!(pair("client_id").as_deref(), Some(CLIENT_ID));
    assert_eq!(pair("redirect_uri").as_deref(), Some(REDIRECT));
    assert_eq!(pair("code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(pair("state").as_deref(), Some(started.state.as_str()));
    assert_eq!(pair("nonce").as_deref(), Some(started.nonce.expose()));
    assert!(pair("scope").is_some_and(|scope| scope.split(' ').any(|s| s == "openid")));
    // An ordinary sign-in asks for no particular freshness.
    assert_eq!(pair("max_age"), None);
    // The verifier itself never travels in the URL; its challenge does.
    assert!(!started.url.contains(started.verifier.expose()));
    // Two sign-ins never share a state, a nonce or a verifier.
    let again = client(&provider, 60).begin(None).await.expect("begun");
    assert_ne!(again.state, started.state);
    assert_ne!(again.nonce.expose(), started.nonce.expose());
    assert_ne!(again.verifier.expose(), started.verifier.expose());
}

#[tokio::test]
async fn a_correct_sign_in_names_the_providers_subject() {
    let provider = Provider::start().await;
    let client = client(&provider, 60);
    assert_eq!(
        sign_in(&provider, &client, "code-good", Shape::Good).await,
        Ok(SUBJECT.to_owned())
    );
}

#[tokio::test]
async fn every_wrong_id_token_is_refused_for_its_own_reason() {
    let provider = Provider::start().await;
    let client = client(&provider, 60);
    let cases = [
        (Shape::WrongNonce, OidcError::IdTokenRefused("nonce")),
        (Shape::WrongAudience, OidcError::IdTokenRefused("audience")),
        (Shape::WrongIssuer, OidcError::IdTokenRefused("issuer")),
        (Shape::Expired, OidcError::IdTokenRefused("expired")),
        (Shape::AlgNone, OidcError::IdTokenRefused("signature")),
        (Shape::Hs256, OidcError::IdTokenRefused("signature")),
        (Shape::WrongAtHash, OidcError::AccessTokenMismatch),
        (Shape::NoSecondFactor, OidcError::NoSecondFactor),
        (Shape::NoIdToken, OidcError::NoIdToken),
    ];
    for (round, (shape, refused)) in cases.into_iter().enumerate() {
        assert_eq!(
            sign_in(&provider, &client, &format!("code-{round}"), shape).await,
            Err(refused),
            "{shape:?}"
        );
    }
}

#[tokio::test]
async fn an_iss_response_parameter_naming_another_issuer_stops_the_sign_in_before_the_exchange() {
    let provider = Provider::start().await;
    let client = client(&provider, 60);
    let started = client.begin(None).await.expect("begun");
    provider.grant(&started.url, "code-mixed", Shape::Good);
    assert_eq!(
        client
            .finish(
                "code-mixed",
                Some("http://127.0.0.1:1"),
                &started.nonce,
                &started.verifier
            )
            .await
            .map(|identity| identity.subject),
        Err(OidcError::IssuerMismatch)
    );
    // The code was never sent: the same code still redeems with the right issuer named.
    assert_eq!(
        client
            .finish(
                "code-mixed",
                Some(&provider.issuer),
                &started.nonce,
                &started.verifier
            )
            .await
            .map(|identity| identity.subject),
        Ok(SUBJECT.to_owned())
    );
}

#[tokio::test]
async fn a_code_redeems_once_and_only_with_its_own_verifier() {
    let provider = Provider::start().await;
    let client = client(&provider, 60);
    let started = client.begin(None).await.expect("begun");
    let other = client.begin(None).await.expect("begun");
    provider.grant(&started.url, "code-pkce", Shape::Good);
    // Another sign-in's verifier is refused by the provider, and that attempt spends the code.
    assert_eq!(
        client
            .finish("code-pkce", None, &started.nonce, &other.verifier)
            .await
            .map(|identity| identity.subject),
        Err(OidcError::CodeRefused)
    );
    provider.grant(&started.url, "code-once", Shape::Good);
    assert_eq!(
        finish(&client, &started, "code-once").await,
        Ok(SUBJECT.to_owned())
    );
    assert_eq!(
        finish(&client, &started, "code-once").await,
        Err(OidcError::CodeRefused)
    );
}

#[tokio::test]
async fn a_new_signing_key_is_fetched_once_when_a_token_names_it() {
    let provider = Provider::start().await;
    let client = client(&provider, 0);
    assert_eq!(
        sign_in(&provider, &client, "code-first", Shape::Good).await,
        Ok(SUBJECT.to_owned())
    );
    assert_eq!(provider.key_fetches(), 1);
    provider.publish_second_key();
    assert_eq!(
        sign_in(&provider, &client, "code-second", Shape::SecondKey).await,
        Ok(SUBJECT.to_owned())
    );
    assert_eq!(provider.key_fetches(), 2, "one refresh for the new key id");
    // Held now: the next token signed with it costs no fetch.
    assert_eq!(
        sign_in(&provider, &client, "code-third", Shape::SecondKey).await,
        Ok(SUBJECT.to_owned())
    );
    assert_eq!(provider.key_fetches(), 2);
}

#[tokio::test]
async fn unknown_key_ids_cannot_make_the_console_fetch_keys_more_than_once_per_interval() {
    let provider = Provider::start().await;
    let client = client(&provider, 60);
    assert_eq!(
        sign_in(&provider, &client, "code-first", Shape::Good).await,
        Ok(SUBJECT.to_owned())
    );
    for round in 0..3 {
        assert_eq!(
            sign_in(
                &provider,
                &client,
                &format!("code-unknown-{round}"),
                Shape::SecondKey
            )
            .await,
            Err(OidcError::IdTokenRefused("signature"))
        );
    }
    assert_eq!(provider.key_fetches(), 1, "no refresh inside the interval");
}

#[tokio::test]
async fn a_fresh_sign_in_asks_for_max_age_and_finish_reports_when_its_holder_authenticated() {
    let provider = Provider::start().await;
    let client = client(&provider, 60);
    let started = client.begin(Some(300)).await.expect("begun");
    let url = openidconnect::url::Url::parse(&started.url).expect("a URL");
    let max_age = url
        .query_pairs()
        .find(|(key, _)| key == "max_age")
        .map(|(_, value)| value.into_owned());
    assert_eq!(max_age.as_deref(), Some("300"));
    let now = chrono::Utc::now().timestamp();
    let authenticated = |shape: Shape, code: &'static str| {
        let client = client.clone();
        let provider = &provider;
        async move {
            let started = client.begin(None).await.expect("begun");
            provider.grant(&started.url, code, shape);
            client
                .finish(code, None, &started.nonce, &started.verifier)
                .await
                .expect("signed in")
                .authenticated
        }
    };
    let fresh = authenticated(Shape::Good, "code-fresh")
        .await
        .expect("auth_time");
    assert!((now - 30..=now).contains(&fresh), "{fresh} vs {now}");
    let stale = authenticated(Shape::StaleAuth, "code-stale")
        .await
        .expect("auth_time");
    assert!(
        (now - 3_700..=now - 3_500).contains(&stale),
        "{stale} vs {now}"
    );
    assert_eq!(authenticated(Shape::NoAuthTime, "code-none").await, None);
}
