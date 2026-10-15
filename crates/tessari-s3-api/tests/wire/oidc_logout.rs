//! A back-channel logout token is believed only as far as an ID token would be: the provider's published key, an
//! algorithm from the console's own list, our issuer and our client, recent, announcing the logout event, naming a
//! subject and carrying no nonce. Every other token is refused with its own reason.

use tessari_s3_infrastructure::OidcSettings;
use tessari_s3_infrastructure::oidc::{OidcClient, OidcError};
use tessari_s3_types::SecretKey;

use crate::oidc_provider::{CLIENT_ID, CLIENT_SECRET, Logout, Provider, REDIRECT, SUBJECT};

fn client(provider: &Provider) -> OidcClient {
    OidcClient::new(
        &OidcSettings {
            issuer: provider.issuer.clone(),
            client_id: CLIENT_ID.to_owned(),
            client_secret: SecretKey::new(CLIENT_SECRET.to_owned()),
            redirect_uri: REDIRECT.to_owned(),
            mfa_amr: vec!["mfa".to_owned()],
            break_glass_webhook: None,
            key_sign_in: tessari_s3_infrastructure::KeySignIn::All,
        },
        60,
    )
    .expect("a client")
}

#[tokio::test]
async fn a_correct_logout_token_names_its_subject_and_its_id() {
    let provider = Provider::start().await;
    let token = provider.logout_token(SUBJECT, "jti-good", Logout::Good);
    let logged_out = client(&provider)
        .verify_logout(&token)
        .await
        .expect("believed");
    assert_eq!(logged_out.issuer, provider.issuer);
    assert_eq!(logged_out.subject, SUBJECT);
    assert_eq!(logged_out.jti, "jti-good");
}

#[tokio::test]
async fn every_wrong_logout_token_is_refused_for_its_own_reason() {
    let provider = Provider::start().await;
    let client = client(&provider);
    let cases = [
        (Logout::WrongIssuer, "issuer"),
        (Logout::WrongAudience, "audience"),
        (Logout::WithNonce, "nonce"),
        (Logout::NoEvent, "event"),
        (Logout::Stale, "issued at"),
        (Logout::OnlySid, "subject"),
        (Logout::Unsigned, "algorithm"),
        (Logout::Hmac, "algorithm"),
        (Logout::UnknownKey, "signature"),
        (Logout::Tampered, "signature"),
    ];
    for (round, (how, reason)) in cases.into_iter().enumerate() {
        let token = provider.logout_token(SUBJECT, &format!("jti-{round}"), how);
        assert_eq!(
            client.verify_logout(&token).await.map(|out| out.subject),
            Err(OidcError::LogoutRefused(reason)),
            "{how:?}"
        );
    }
    for junk in ["", "a.b", "a.b.c.d", "!!!.???.***"] {
        assert_eq!(
            client.verify_logout(junk).await.map(|out| out.subject),
            Err(OidcError::LogoutRefused("malformed")),
            "{junk}"
        );
    }
}
