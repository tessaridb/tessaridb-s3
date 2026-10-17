//! Users as the rest of the server sees them, what creating one takes, and what issuing or resolving a key answers.

use tessari_s3_core::authz::{Role, SpaceName, UserName};
use tessari_s3_types::{BucketName, ObjectKey, SecretKey, Timestamp};

use super::entity::{AccessKeyEntity, UserEntity};
use super::sealer::{ScopeBinding, SessionBinding};
use crate::{Error, Result};

/// How a role is stored.
pub(crate) const fn role_text(role: Role) -> &'static str {
    match role {
        Role::SpaceAdmin => "admin",
        Role::Member => "member",
    }
}

/// One user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// Its name.
    pub name: UserName,
    /// The space it belongs to.
    pub space: SpaceName,
    /// Its role in that space.
    pub role: Role,
    /// Whether a member may create buckets in the space.
    pub create_buckets: bool,
    /// Store-wide: every space, users, quotas.
    pub operator: bool,
    /// Store-wide: the cluster's members, drives and healing.
    pub cluster_viewer: bool,
    /// A disabled user's keys resolve to nothing.
    pub disabled: bool,
    /// When it was created.
    pub created: Timestamp,
}

/// What creating a user takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUser {
    /// Its name, unique across the store.
    pub name: UserName,
    /// The space it joins; it must exist.
    pub space: SpaceName,
    /// Its role there.
    pub role: Role,
    /// Whether a member may create buckets in the space.
    pub create_buckets: bool,
    /// Store-wide operator.
    pub operator: bool,
    /// Store-wide cluster viewer.
    pub cluster_viewer: bool,
}

/// A key just issued: the only time its secret is shown. Not `Clone`: every copy of the secret is a deliberate act.
#[derive(Debug)]
pub struct IssuedKey {
    /// The access key id, `TS` and 18 more characters.
    pub access_key_id: String,
    /// The secret, 40 characters; stored only sealed.
    pub secret: SecretKey,
}

/// What a one-key credential may do: upload one key of one bucket, until it expires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyScope {
    /// The bucket it may write.
    pub bucket: BucketName,
    /// The one key it may write.
    pub key: ObjectKey,
    /// When it stops working, in whole seconds since the epoch.
    pub expires: i64,
}

impl KeyScope {
    /// The scope as bound into the sealed secret's associated data.
    pub(crate) fn binding(&self) -> ScopeBinding<'_> {
        ScopeBinding {
            bucket: self.bucket.as_str(),
            key: self.key.as_str(),
            expires: self.expires,
        }
    }
}

/// Temporary credentials issued through the identity provider: until when, the digest of the token they are presented
/// with, and the identity they were issued to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionScope {
    /// When they stop working, in whole seconds since the epoch.
    pub expires: i64,
    /// The SHA-256 digest of the session token.
    pub token_digest: [u8; 32],
    /// The provider's issuer.
    pub issuer: String,
    /// The provider's subject.
    pub subject: String,
    /// When they were issued, in whole seconds since the epoch.
    pub issued: i64,
    /// The session policy they were issued with, as written; `None` for none.
    pub policy: Option<String>,
}

impl SessionScope {
    /// The session as bound into the sealed secret's associated data.
    pub(crate) fn binding(&self) -> SessionBinding<'_> {
        SessionBinding {
            expires: self.expires,
            token_digest: &self.token_digest,
            issuer: &self.issuer,
            subject: &self.subject,
            policy: self.policy.as_deref(),
        }
    }
}

/// What issuing temporary credentials takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    /// When they stop working, in whole seconds since the epoch.
    pub expires: i64,
    /// The SHA-256 digest of the session token they will be presented with.
    pub token_digest: [u8; 32],
    /// The provider's issuer the identity token came from.
    pub issuer: String,
    /// The provider's subject it named.
    pub subject: String,
    /// The session policy narrowing them, as written (already checked); `None` for none.
    pub policy: Option<String>,
}

/// A key record's scope: all three fields or none of them; anything between is a malformed record, never a full key.
/// A session record carries an expiry and none of the other two.
pub(crate) fn scope_of(key: &AccessKeyEntity) -> Result<Option<KeyScope>> {
    scope_from(
        key.scope_bucket.as_ref(),
        key.scope_key.as_ref(),
        key.expires,
        key.session_hash.is_some(),
    )
}

/// A key record's session: the token digest, issuer, subject and expiry together, or none of them.
pub(crate) fn session_of(key: &AccessKeyEntity) -> Result<Option<SessionScope>> {
    let malformed = |reason| Error::Malformed {
        record: "access key",
        reason,
    };
    match (
        key.session_hash.as_ref(),
        key.session_issuer.as_ref(),
        key.session_subject.as_ref(),
        key.expires,
    ) {
        (None, None, None, _) => Ok(None),
        (Some(hash), Some(issuer), Some(subject), Some((expires, _))) => Ok(Some(SessionScope {
            expires,
            token_digest: hash
                .as_slice()
                .try_into()
                .map_err(|_| malformed("session_hash"))?,
            issuer: issuer.clone(),
            subject: subject.clone(),
            issued: key.created.0,
            policy: key.session_policy.clone(),
        })),
        _ => Err(malformed(
            "a session needs session_hash, session_issuer, session_subject and expires together",
        )),
    }
}

/// A scope from its three stored fields, under the same all-or-none rule as [`scope_of`]; a session's record
/// (`session`) holds its expiry alone.
pub(crate) fn scope_from(
    bucket: Option<&String>,
    key: Option<&String>,
    expires: Option<(i64, u32)>,
    session: bool,
) -> Result<Option<KeyScope>> {
    let malformed = |reason| Error::Malformed {
        record: "access key",
        reason,
    };
    match (bucket, key, expires) {
        (None, None, None) => Ok(None),
        (None, None, Some(_)) if session => Ok(None),
        (Some(bucket), Some(object), Some((expires, _))) => Ok(Some(KeyScope {
            bucket: BucketName::new(bucket).map_err(|_| malformed("scope_bucket"))?,
            key: ObjectKey::new(object).map_err(|_| malformed("scope_key"))?,
            expires,
        })),
        _ => Err(malformed(
            "a scope needs scope_bucket, scope_key and expires together",
        )),
    }
}

/// The token a console form carries so that submitting it twice issues one key, not two: 16 to 64 characters of
/// ASCII letters, digits and `-` (a UUID fits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueToken(String);

impl IssueToken {
    /// The token `text` spells; `None` for any other shape.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        let fits = (16..=64).contains(&text.len())
            && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        fits.then(|| Self(text.to_owned()))
    }

    /// The token as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What issuing a key under a token ended in.
#[derive(Debug)]
pub enum IssuedOnce {
    /// A new key; its secret is in this answer and nowhere else.
    Issued(IssuedKey),
    /// The token already issued this user a key: its id, and no secret.
    AlreadyIssued(String),
    /// The token issued a key to another user; nothing was issued.
    TokenReused,
    /// There is no such user.
    NoSuchUser,
}

/// One of a user's keys as a listing shows it: never its secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedKey {
    /// The access key id.
    pub access_key_id: String,
    /// When it was issued.
    pub created: Timestamp,
    /// A disabled key resolves to nothing.
    pub disabled: bool,
    /// A one-key credential's scope; `None` for a full key.
    pub scope: Option<KeyScope>,
    /// Until when temporary credentials last; `None` for any other key.
    pub session_expires: Option<i64>,
}

/// A live key's user and secret, for verifying a request it signed. Not `Clone`, for the same reason.
#[derive(Debug)]
pub struct Resolved {
    /// The user holding the key — for a one-key credential, the user who issued it.
    pub user: User,
    /// The key's secret.
    pub secret: SecretKey,
    /// A one-key credential's scope; `None` for a user's full key.
    pub scope: Option<KeyScope>,
    /// Temporary credentials' session; `None` for any other key.
    pub session: Option<SessionScope>,
}

impl TryFrom<UserEntity> for User {
    type Error = Error;

    fn try_from(entity: UserEntity) -> Result<Self> {
        let malformed = |reason| Error::Malformed {
            record: "user",
            reason,
        };
        let name = UserName::new(&entity.name).ok_or(malformed("name is not a valid user name"))?;
        let space = SpaceName::new(&entity.space_name)
            .ok_or(malformed("space_name is not a valid space name"))?;
        let role = match entity.space_role.as_str() {
            "admin" => Role::SpaceAdmin,
            "member" => Role::Member,
            _ => return Err(malformed("space_role")),
        };
        let (seconds, nanos) = entity.created;
        Ok(Self {
            name,
            space,
            role,
            create_buckets: entity.create_buckets,
            operator: entity.operator,
            cluster_viewer: entity.cluster_viewer,
            disabled: entity.disabled,
            created: Timestamp { seconds, nanos },
        })
    }
}
