//! Users, access keys and grants in TessariDB: `users:<name>`, `access_keys:<access key id>` and
//! `grants:<user>/<bucket>` (neither name may hold a `/`); every value is bound.

use tessari_s3_core::authz::{Access, UserName, Visible};
use tessari_s3_infrastructure::tessaridb::{MetaPool, RefusalClass, Value};
use tessari_s3_types::BucketName;

use super::entity::{AccessKeyEntity, GrantEntity, UserEntity};
use super::model::{NewUser, role_text};
use super::repository::{Inserted, UserRepository};
use super::sealer::Sealed;
use crate::answers::{all_records, first_record};
use crate::{Error, Result};

/// The TessariDB user repository.
#[derive(Clone)]
pub struct TessariUsers {
    pool: MetaPool,
}

impl TessariUsers {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }

    /// The first record a single-statement script answers, if any.
    async fn one(&self, script: &str, parameters: Vec<(String, Value)>) -> Result<Option<Value>> {
        let answers = self.pool.run(script, parameters).await?;
        first_record(answers.into_iter().next())
    }

    /// Every record a single-statement read answers.
    async fn all(
        &self,
        script: &str,
        parameters: Vec<(String, Value)>,
        record: &'static str,
    ) -> Result<Vec<Value>> {
        let answers = self.pool.run(script, parameters).await?;
        all_records(answers.into_iter().next(), record)
    }
}

fn text(name: &str, value: &str) -> (String, Value) {
    (name.to_owned(), Value::String(value.to_owned()))
}

fn flag(name: &str, value: bool) -> (String, Value) {
    (name.to_owned(), Value::Bool(value))
}

fn grant_id(user: &UserName, bucket: &BucketName) -> (String, Value) {
    text("id", &format!("{}/{}", user.as_str(), bucket.as_str()))
}

impl UserRepository for TessariUsers {
    async fn insert(&self, user: &NewUser) -> Result<Inserted> {
        let parameters = vec![
            text("name", user.name.as_str()),
            text("space", user.space.as_str()),
            text("role", role_text(user.role)),
            flag("create_buckets", user.create_buckets),
            flag("operator", user.operator),
            flag("cluster_viewer", user.cluster_viewer),
        ];
        let script = "CREATE users:$name = { name: $name, space_name: $space, space_role: $role, \
                      create_buckets: $create_buckets, operator: $operator, cluster_viewer: $cluster_viewer, \
                      disabled: false, created: time::now() } RETURN AFTER;";
        match self.one(script, parameters).await {
            Ok(Some(value)) => Ok(Inserted::Created(UserEntity::from_value(&value)?)),
            Ok(None) => Err(Error::Malformed {
                record: "user",
                reason: "CREATE answered no record",
            }),
            Err(Error::Meta(error)) if error.is_class(RefusalClass::Conflict) => {
                Ok(Inserted::Exists)
            }
            Err(error) => Err(error),
        }
    }

    async fn get(&self, name: &UserName) -> Result<Option<UserEntity>> {
        self.one(
            "SELECT * FROM ONLY users:$name;",
            vec![text("name", name.as_str())],
        )
        .await?
        .map(|value| UserEntity::from_value(&value))
        .transpose()
    }

    async fn list(&self, visible: &Visible) -> Result<Vec<UserEntity>> {
        let records = match visible {
            Visible::All => self.all("SELECT * FROM users;", Vec::new(), "user").await?,
            Visible::Space(space) => {
                self.all(
                    "SELECT * FROM users WHERE space_name = $space USING INDEX by_user_space;",
                    vec![text("space", space.as_str())],
                    "user",
                )
                .await?
            }
        };
        records.iter().map(UserEntity::from_value).collect()
    }

    async fn set_disabled(&self, name: &UserName, disabled: bool) -> Result<()> {
        let parameters = vec![text("name", name.as_str()), flag("disabled", disabled)];
        self.pool
            .run("UPDATE users:$name SET disabled = $disabled;", parameters)
            .await?;
        Ok(())
    }

    async fn insert_key(&self, id: &str, user: &UserName, sealed: &Sealed) -> Result<bool> {
        let parameters = vec![
            text("id", id),
            text("user", user.as_str()),
            ("secret".to_owned(), Value::Bytes(sealed.ciphertext.clone())),
            ("nonce".to_owned(), Value::Bytes(sealed.nonce.clone())),
            text("kek_id", &sealed.kek_id),
            text("algorithm", &sealed.algorithm),
        ];
        let script = "CREATE access_keys:$id = { user_name: $user, secret: $secret, nonce: $nonce, \
                      kek_id: $kek_id, algorithm: $algorithm, disabled: false, created: time::now() };";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn get_key(&self, id: &str) -> Result<Option<AccessKeyEntity>> {
        self.one("SELECT * FROM ONLY access_keys:$id;", vec![text("id", id)])
            .await?
            .map(|value| AccessKeyEntity::from_value(&value))
            .transpose()
    }

    async fn set_key_disabled(&self, id: &str, disabled: bool) -> Result<()> {
        let parameters = vec![text("id", id), flag("disabled", disabled)];
        self.pool
            .run(
                "UPDATE access_keys:$id SET disabled = $disabled;",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn remove_key(&self, id: &str) -> Result<bool> {
        let removed = self
            .one(
                "DELETE access_keys:$id RETURN BEFORE;",
                vec![text("id", id)],
            )
            .await?;
        Ok(removed.is_some())
    }

    async fn put_grant(&self, user: &UserName, bucket: &BucketName, access: Access) -> Result<()> {
        let parameters = vec![
            grant_id(user, bucket),
            text("user", user.as_str()),
            text("bucket", bucket.as_str()),
            flag("can_read", access.read),
            flag("can_write", access.write),
        ];
        let script = "UPSERT grants:$id = { user_name: $user, bucket_name: $bucket, can_read: $can_read, \
                      can_write: $can_write };";
        self.pool.run(script, parameters).await?;
        Ok(())
    }

    async fn remove_grant(&self, user: &UserName, bucket: &BucketName) -> Result<bool> {
        let removed = self
            .one(
                "DELETE grants:$id RETURN BEFORE;",
                vec![grant_id(user, bucket)],
            )
            .await?;
        Ok(removed.is_some())
    }

    async fn grants(&self, user: &UserName) -> Result<Vec<GrantEntity>> {
        self.all(
            "SELECT * FROM grants WHERE user_name = $user USING INDEX by_grant_user;",
            vec![text("user", user.as_str())],
            "grant",
        )
        .await?
        .iter()
        .map(GrantEntity::from_value)
        .collect()
    }
}
