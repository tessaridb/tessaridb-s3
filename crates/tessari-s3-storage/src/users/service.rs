//! User rules: creating users in an existing space, issuing and resolving their keys, granting buckets, and the
//! principal the access evaluator decides for.

use std::collections::BTreeMap;
use std::sync::Arc;

use tessari_s3_core::authz::{Access, UserName, UserPrincipal, Visible};
use tessari_s3_types::{BucketName, IamKey};

use super::credentials;
use super::model::{IssuedKey, NewUser, Resolved, User};
use super::repository::{Inserted, UserRepository};
use super::sealer::{Binding, Sealed, Sealer};
use super::tessaridb::TessariUsers;
use crate::spaces::SpaceService;
use crate::{Error, Result};

/// Draws of a fresh access key id before giving up; a collision among 90-bit ids is not expected even once.
const ID_ATTEMPTS: usize = 3;

/// What creating a user ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserCreated {
    /// The user now exists, enabled.
    Created(User),
    /// A user of that name already existed.
    Exists,
    /// The space it was to join does not exist.
    NoSuchSpace,
}

/// User, access-key and grant operations.
#[derive(Clone)]
pub struct UserService {
    repository: TessariUsers,
    spaces: SpaceService,
    sealer: Option<Arc<Sealer>>,
}

impl UserService {
    /// The service over `repository`, checking spaces through `spaces`; it seals nothing until given a root key.
    #[must_use]
    pub const fn new(repository: TessariUsers, spaces: SpaceService) -> Self {
        Self {
            repository,
            spaces,
            sealer: None,
        }
    }

    /// The same service sealing and opening access-key secrets under `root`; `None` leaves it unable to issue or
    /// resolve a key.
    #[must_use]
    pub fn with_iam_key(self, root: Option<IamKey>) -> Self {
        Self {
            sealer: root.map(|root| Arc::new(Sealer::new(root))),
            ..self
        }
    }

    fn sealer(&self) -> Result<&Sealer> {
        self.sealer.as_deref().ok_or(Error::NoIamKey)
    }

    /// Creates `user`, enabled, in its space.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn create(&self, user: &NewUser) -> Result<UserCreated> {
        if self.spaces.get(&user.space).await?.is_none() {
            return Ok(UserCreated::NoSuchSpace);
        }
        match self.repository.insert(user).await? {
            Inserted::Created(entity) => Ok(UserCreated::Created(User::try_from(entity)?)),
            Inserted::Exists => Ok(UserCreated::Exists),
        }
    }

    /// The user called `name`, if it exists.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn get(&self, name: &UserName) -> Result<Option<User>> {
        self.repository
            .get(name)
            .await?
            .map(User::try_from)
            .transpose()
    }

    /// The users `visible` covers, in byte order of their names.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn list(&self, visible: &Visible) -> Result<Vec<User>> {
        let mut users: Vec<User> = self
            .repository
            .list(visible)
            .await?
            .into_iter()
            .map(User::try_from)
            .collect::<Result<_>>()?;
        users.sort_by(|a, b| a.name.as_str().as_bytes().cmp(b.name.as_str().as_bytes()));
        Ok(users)
    }

    /// Disables or enables `name`; `false` when there is no such user.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_disabled(&self, name: &UserName, disabled: bool) -> Result<bool> {
        if self.repository.get(name).await?.is_none() {
            return Ok(false);
        }
        self.repository.set_disabled(name, disabled).await?;
        Ok(true)
    }

    /// A new key for `user`, its secret shown here once and stored only sealed; `None` when there is no such user.
    ///
    /// # Errors
    /// [`Error::NoIamKey`] without a root key; [`Error::Randomness`]; the metadata store's refusal or outage.
    pub async fn issue_key(&self, user: &UserName) -> Result<Option<IssuedKey>> {
        let sealer = self.sealer()?;
        let Some(holder) = self.get(user).await? else {
            return Ok(None);
        };
        let secret = credentials::secret()?;
        for _ in 0..ID_ATTEMPTS {
            let id = credentials::access_key_id()?;
            let binding = Binding {
                key_id: &id,
                user: holder.name.as_str(),
                space: holder.space.as_str(),
            };
            let sealed = sealer.seal(&secret, binding)?;
            if self
                .repository
                .insert_key(&id, &holder.name, &sealed)
                .await?
            {
                return Ok(Some(IssuedKey {
                    access_key_id: id,
                    secret,
                }));
            }
        }
        Err(Error::Randomness)
    }

    /// The user and secret of key `id` while the key and its user are both enabled; `None` otherwise, alike for a
    /// key that does not exist, so a caller cannot tell which. An id of a shape never issued is answered without a
    /// lookup.
    ///
    /// # Errors
    /// [`Error::Unsealable`] when the stored secret does not open (another root key, a record moved or changed);
    /// [`Error::NoIamKey`] without a root key; the metadata store's refusal or outage.
    pub async fn resolve(&self, id: &str) -> Result<Option<Resolved>> {
        if !credentials::is_access_key_id(id) {
            return Ok(None);
        }
        let Some(key) = self.repository.get_key(id).await? else {
            return Ok(None);
        };
        if key.disabled {
            return Ok(None);
        }
        let Some(name) = UserName::new(&key.user_name) else {
            return Err(Error::Malformed {
                record: "access key",
                reason: "user_name is not a valid user name",
            });
        };
        let Some(user) = self.get(&name).await?.filter(|user| !user.disabled) else {
            return Ok(None);
        };
        let sealed = Sealed {
            ciphertext: key.secret,
            nonce: key.nonce,
            kek_id: key.kek_id,
            algorithm: key.algorithm,
        };
        let binding = Binding {
            key_id: id,
            user: user.name.as_str(),
            space: user.space.as_str(),
        };
        let secret = self.sealer()?.open(&sealed, binding)?;
        Ok(Some(Resolved { user, secret }))
    }

    /// Disables or enables key `id`; `false` when there is no such key.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_key_disabled(&self, id: &str, disabled: bool) -> Result<bool> {
        if self.repository.get_key(id).await?.is_none() {
            return Ok(false);
        }
        self.repository.set_key_disabled(id, disabled).await?;
        Ok(true)
    }

    /// Removes key `id`; `false` when there was none.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn revoke_key(&self, id: &str) -> Result<bool> {
        self.repository.remove_key(id).await
    }

    /// Grants `user` `access` to `bucket`, replacing what it had there; `false` when there is no such user.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_grant(
        &self,
        user: &UserName,
        bucket: &BucketName,
        access: Access,
    ) -> Result<bool> {
        if self.repository.get(user).await?.is_none() {
            return Ok(false);
        }
        self.repository.put_grant(user, bucket, access).await?;
        Ok(true)
    }

    /// Removes `user`'s grant on `bucket`; `false` when there was none.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn remove_grant(&self, user: &UserName, bucket: &BucketName) -> Result<bool> {
        self.repository.remove_grant(user, bucket).await
    }

    /// `name` as the access evaluator sees it, with its grants; `None` when it does not exist or is disabled.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn principal(&self, name: &UserName) -> Result<Option<UserPrincipal>> {
        let Some(user) = self.get(name).await?.filter(|user| !user.disabled) else {
            return Ok(None);
        };
        let grants: BTreeMap<String, Access> = self
            .repository
            .grants(name)
            .await?
            .into_iter()
            .map(|grant| {
                let access = Access {
                    read: grant.can_read,
                    write: grant.can_write,
                };
                (grant.bucket_name, access)
            })
            .collect();
        Ok(Some(UserPrincipal {
            name: user.name.as_str().to_owned(),
            space: user.space,
            role: user.role,
            create_buckets: user.create_buckets,
            operator: user.operator,
            cluster_viewer: user.cluster_viewer,
            grants,
        }))
    }
}
