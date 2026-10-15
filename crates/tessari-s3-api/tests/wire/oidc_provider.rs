//! An identity provider for the tests, served on a loopback port: discovery, keys, and a token endpoint that holds
//! each code to the PKCE challenge it was granted under and redeems it once. Each code answers in a SHAPE the test
//! chose — a correct ID token, or one of the ways a token can be wrong — so a refusal is asserted against a real
//! exchange rather than against a hand-built value. Its signing keys are Ed25519, derived from fixed seeds: no RSA
//! private-key operation anywhere, and no key material in the repository.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use openidconnect::core::{
    CoreEdDsaPrivateSigningKey, CoreHmacKey, CoreIdToken, CoreIdTokenClaims, CoreJsonWebKeySet,
    CoreJwsSigningAlgorithm,
};
use openidconnect::{
    AccessToken, Audience, AuthenticationMethodReference, EmptyAdditionalClaims, IssuerUrl,
    JsonWebKeyId, Nonce, PrivateSigningKey, StandardClaims, SubjectIdentifier, url,
};
use sha2::{Digest, Sha256};

pub(crate) const CLIENT_ID: &str = "console-test";
pub(crate) const CLIENT_SECRET: &str = "console-test-secret-0123456789abcdef";
pub(crate) const REDIRECT: &str = "http://127.0.0.1:47999/api/v1/auth/oidc/callback";
pub(crate) const SUBJECT: &str = "subject-of-ann";
/// The key the provider publishes from the start, and the one it can publish later.
const FIRST: Key = ("k1", [1; 32]);
const SECOND: Key = ("k2", [2; 32]);

/// A signing key: its id and the seed it is derived from.
type Key = (&'static str, [u8; 32]);

/// The event a back-channel logout token announces.
const LOGOUT_EVENT: &str = "http://schemas.openid.net/event/backchannel-logout";

/// How a back-channel logout token is made, right or in one of the ways it can be wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Logout {
    /// Signed with the published key, every claim as the specification requires.
    Good,
    /// Naming another issuer.
    WrongIssuer,
    /// Addressed to another client.
    WrongAudience,
    /// Carrying a `nonce`, which a logout token must not.
    WithNonce,
    /// Without the back-channel logout event.
    NoEvent,
    /// Issued ten minutes ago.
    Stale,
    /// Naming a provider session (`sid`) and no subject.
    OnlySid,
    /// `alg: none`, unsigned.
    Unsigned,
    /// HMAC-signed with the client's own secret.
    Hmac,
    /// Signed with the second key, which the provider has not published.
    UnknownKey,
    /// Signed with the published key, then one byte of the claims changed.
    Tampered,
}

/// How the token endpoint answers a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A correct ID token, with a second factor in `amr`, authenticated five seconds ago.
    Good,
    /// Correct, but its holder authenticated an hour ago.
    StaleAuth,
    /// Correct, but it does not say when its holder authenticated.
    NoAuthTime,
    /// Correct, but `amr` names a password only.
    NoSecondFactor,
    /// Signed for another nonce.
    WrongNonce,
    /// Addressed to another client.
    WrongAudience,
    /// Naming another issuer.
    WrongIssuer,
    /// Past its expiry.
    Expired,
    /// `alg: none`, unsigned.
    AlgNone,
    /// HMAC-signed with the client's own secret.
    Hs256,
    /// Signed with the second key, which the provider may or may not publish yet.
    SecondKey,
    /// `at_hash` of another access token.
    WrongAtHash,
    /// A token response with no ID token.
    NoIdToken,
}

struct Grant {
    challenge: String,
    nonce: String,
    shape: Shape,
}

#[derive(Clone)]
struct Shared {
    issuer: String,
    grants: Arc<Mutex<HashMap<String, Grant>>>,
    published: Arc<Mutex<Vec<Key>>>,
    key_fetches: Arc<AtomicUsize>,
}

/// A running provider; stopped when dropped.
pub(crate) struct Provider {
    pub(crate) issuer: String,
    shared: Shared,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn pem(seed: [u8; 32]) -> String {
    // PKCS#8 for an Ed25519 key: a fixed 16-byte prefix and the 32-byte seed.
    let mut der = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    der.extend_from_slice(&seed);
    format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        STANDARD.encode(der)
    )
}

fn signing_key((kid, seed): (&str, [u8; 32])) -> CoreEdDsaPrivateSigningKey {
    CoreEdDsaPrivateSigningKey::from_ed25519_pem(
        &pem(seed),
        Some(JsonWebKeyId::new(kid.to_owned())),
    )
    .expect("an Ed25519 key")
}

impl Provider {
    pub(crate) async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let issuer = format!("http://{}", listener.local_addr().expect("address"));
        let shared = Shared {
            issuer: issuer.clone(),
            grants: Arc::default(),
            published: Arc::new(Mutex::new(vec![FIRST])),
            key_fetches: Arc::default(),
        };
        let app = axum::Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/keys", get(keys))
            .route("/token", post(token))
            .with_state(shared.clone());
        let task = tokio::spawn(async move {
            let _ended = axum::serve(listener, app).await;
        });
        Self {
            issuer,
            shared,
            task,
        }
    }

    /// Grants `code` to the sign-in `authorization_url` started, answering in `shape`.
    pub(crate) fn grant(&self, authorization_url: &str, code: &str, shape: Shape) {
        let url = url::Url::parse(authorization_url).expect("a URL");
        let pair = |name: &str| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
                .unwrap_or_else(|| panic!("the authorization URL carries {name}"))
        };
        self.grant_with(code, &pair("code_challenge"), &pair("nonce"), shape);
    }

    /// Grants `code` under an explicit `challenge` and `nonce`.
    pub(crate) fn grant_with(&self, code: &str, challenge: &str, nonce: &str, shape: Shape) {
        self.shared.grants.lock().expect("grants").insert(
            code.to_owned(),
            Grant {
                challenge: challenge.to_owned(),
                nonce: nonce.to_owned(),
                shape,
            },
        );
    }

    /// Starts publishing the second key beside the first.
    pub(crate) fn publish_second_key(&self) {
        self.shared.published.lock().expect("keys").push(SECOND);
    }

    /// A back-channel logout token for `subject` with id `jti`, made as `how` says.
    pub(crate) fn logout_token(&self, subject: &str, jti: &str, how: Logout) -> String {
        let now = Utc::now().timestamp();
        let mut claims = serde_json::json!({
            "iss": if how == Logout::WrongIssuer { "http://127.0.0.1:1" } else { self.issuer.as_str() },
            "aud": if how == Logout::WrongAudience { "another-client" } else { CLIENT_ID },
            "iat": if how == Logout::Stale { now.saturating_sub(600) } else { now },
            "jti": jti,
            "sub": subject,
            "events": { LOGOUT_EVENT: {} },
        });
        if how == Logout::WithNonce {
            claims["nonce"] = serde_json::Value::String("a-nonce".to_owned());
        }
        if how == Logout::NoEvent {
            claims["events"] = serde_json::json!({});
        }
        if how == Logout::OnlySid {
            claims["sid"] = serde_json::Value::String("a-session".to_owned());
            if let Some(object) = claims.as_object_mut() {
                object.remove("sub");
            }
        }
        let (alg, kid) = match how {
            Logout::Unsigned => ("none", None),
            Logout::Hmac => ("HS256", None),
            Logout::UnknownKey => ("EdDSA", Some(SECOND.0)),
            _ => ("EdDSA", Some(FIRST.0)),
        };
        let mut header = serde_json::json!({ "alg": alg, "typ": "logout+jwt" });
        if let Some(kid) = kid {
            header["kid"] = serde_json::Value::String(kid.to_owned());
        }
        let encode = |value: &serde_json::Value| {
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).expect("JSON"))
        };
        let input = format!("{}.{}", encode(&header), encode(&claims));
        let signature = match how {
            Logout::Unsigned => Vec::new(),
            Logout::Hmac => CoreHmacKey::new(CLIENT_SECRET)
                .sign(&CoreJwsSigningAlgorithm::HmacSha256, input.as_bytes())
                .expect("signed"),
            Logout::UnknownKey => signing_key(SECOND)
                .sign(&CoreJwsSigningAlgorithm::EdDsa, input.as_bytes())
                .expect("signed"),
            _ => signing_key(FIRST)
                .sign(&CoreJwsSigningAlgorithm::EdDsa, input.as_bytes())
                .expect("signed"),
        };
        let token = format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature));
        if how == Logout::Tampered {
            // Re-encode the claims with another subject under the original signature.
            let mut parts = token.splitn(3, '.');
            let (head, _, mac) = (
                parts.next().expect("header"),
                parts.next().expect("claims"),
                parts.next().expect("signature"),
            );
            claims["sub"] = serde_json::Value::String("someone-else".to_owned());
            return format!("{head}.{}.{mac}", encode(&claims));
        }
        token
    }

    /// How many times the key set was fetched.
    pub(crate) fn key_fetches(&self) -> usize {
        self.shared.key_fetches.load(Ordering::Acquire)
    }
}

async fn discovery(State(shared): State<Shared>) -> Json<serde_json::Value> {
    // Shape known per test; this document is the provider's own, and it advertises algorithms the console must not
    // take from it.
    Json(serde_json::json!({
        "issuer": shared.issuer,
        "authorization_endpoint": format!("{}/authorize", shared.issuer),
        "token_endpoint": format!("{}/token", shared.issuer),
        "jwks_uri": format!("{}/keys", shared.issuer),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["EdDSA", "HS256", "none"],
    }))
}

async fn keys(State(shared): State<Shared>) -> Json<CoreJsonWebKeySet> {
    shared.key_fetches.fetch_add(1, Ordering::AcqRel);
    let published = shared.published.lock().expect("keys").clone();
    Json(CoreJsonWebKeySet::new(
        published
            .into_iter()
            .map(|key| signing_key(key).as_verification_key())
            .collect(),
    ))
}

fn refuse() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "invalid_grant"})),
    )
        .into_response()
}

async fn token(State(shared): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    let basic = format!(
        "Basic {}",
        STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"))
    );
    let authenticated = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        == Some(basic.as_str());
    let field = |name: &str| form.get(name).map(String::as_str);
    if !authenticated
        || field("grant_type") != Some("authorization_code")
        || field("redirect_uri") != Some(REDIRECT)
    {
        return refuse();
    }
    // Redeemed once: the grant leaves the map whatever happens next.
    let Some(grant) =
        field("code").and_then(|code| shared.grants.lock().expect("grants").remove(code))
    else {
        return refuse();
    };
    let challenge = field("code_verifier")
        .map(|verifier| URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())));
    if challenge.as_deref() != Some(grant.challenge.as_str()) {
        return refuse();
    }
    let access = AccessToken::new(format!("access-{}", grant.nonce));
    let mut answer = serde_json::json!({
        "access_token": access.secret(),
        "token_type": "Bearer",
        "expires_in": 300,
    });
    if grant.shape != Shape::NoIdToken {
        answer["id_token"] = serde_json::Value::String(id_token(&shared.issuer, &grant, &access));
    }
    Json(answer).into_response()
}

fn id_token(issuer: &str, grant: &Grant, access: &AccessToken) -> String {
    let now = Utc::now();
    let at = |minutes: i64| {
        now.checked_add_signed(Duration::minutes(minutes))
            .expect("an instant in range")
    };
    let (issuer, audience, nonce, expiry) = match grant.shape {
        Shape::WrongIssuer => (
            "http://127.0.0.1:1".to_owned(),
            CLIENT_ID,
            grant.nonce.clone(),
            at(5),
        ),
        Shape::WrongAudience => (
            issuer.to_owned(),
            "another-client",
            grant.nonce.clone(),
            at(5),
        ),
        Shape::WrongNonce => (
            issuer.to_owned(),
            CLIENT_ID,
            "another-nonce".to_owned(),
            at(5),
        ),
        Shape::Expired => (issuer.to_owned(), CLIENT_ID, grant.nonce.clone(), at(-5)),
        _ => (issuer.to_owned(), CLIENT_ID, grant.nonce.clone(), at(5)),
    };
    let factors: &[&str] = if grant.shape == Shape::NoSecondFactor {
        &["pwd"]
    } else {
        &["pwd", "mfa"]
    };
    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new(issuer).expect("an issuer"),
        vec![Audience::new(audience.to_owned())],
        expiry,
        at(-10),
        StandardClaims::new(SubjectIdentifier::new(SUBJECT.to_owned())),
        EmptyAdditionalClaims {},
    )
    .set_nonce(Some(Nonce::new(nonce)))
    .set_auth_time(match grant.shape {
        Shape::NoAuthTime => None,
        Shape::StaleAuth => Some(at(-60)),
        _ => Some(
            now.checked_sub_signed(Duration::seconds(5))
                .expect("an instant in range"),
        ),
    })
    .set_auth_method_refs(Some(
        factors
            .iter()
            .map(|factor| AuthenticationMethodReference::new((*factor).to_owned()))
            .collect(),
    ));
    let other = AccessToken::new("another-access-token".to_owned());
    let hashed = if grant.shape == Shape::WrongAtHash {
        &other
    } else {
        access
    };
    let signed = match grant.shape {
        Shape::AlgNone => {
            let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
            let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims"));
            return format!("{header}.{payload}.");
        }
        // No `at_hash`: its check would refuse this token for want of a key, and the test is about the verifier.
        Shape::Hs256 => CoreIdToken::new(
            claims,
            &CoreHmacKey::new(CLIENT_SECRET),
            CoreJwsSigningAlgorithm::HmacSha256,
            None,
            None,
        ),
        Shape::SecondKey => CoreIdToken::new(
            claims,
            &signing_key(SECOND),
            CoreJwsSigningAlgorithm::EdDsa,
            Some(hashed),
            None,
        ),
        _ => CoreIdToken::new(
            claims,
            &signing_key(FIRST),
            CoreJwsSigningAlgorithm::EdDsa,
            Some(hashed),
            None,
        ),
    };
    signed.expect("a signed ID token").to_string()
}
