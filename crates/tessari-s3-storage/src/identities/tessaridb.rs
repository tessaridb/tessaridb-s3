//! Logins and identity links in TessariDB: `oidc_logins:<state>`, dropped by the store at its expiry, and
//! `identity_links:<id>`, the id derived from the issuer and the subject by the service; every value is bound.

use tessari_s3_core::authz::UserName;
use tessari_s3_infrastructure::tessaridb::{MetaPool, RefusalClass, Value};

use super::entity::{LinkEntity, LoginEntity, logout_after};
use super::model::PendingLogin;
use super::repository::{IdentityRepository, LinkWritten};
use crate::Result;
use crate::answers::{all_records, first_record};

/// The TessariDB identities repository.
#[derive(Clone)]
pub struct TessariIdentities {
    pool: MetaPool,
}

impl TessariIdentities {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn text(name: &str, value: &str) -> (String, Value) {
    (name.to_owned(), Value::String(value.to_owned()))
}

/// Whether a refusal means a concurrent write won: the loser of an overlap is `retry`, of an earlier commit (or a
/// record already there) `conflict`; either way it wrote nothing.
fn lost_race(error: &tessari_s3_infrastructure::tessaridb::MetaError) -> bool {
    error.is_class(RefusalClass::Conflict) || error.is_class(RefusalClass::Retry)
}

impl IdentityRepository for TessariIdentities {
    async fn insert_login(&self, login: &PendingLogin, lifetime_secs: i64) -> Result<()> {
        let parameters = vec![
            text("state", &login.state),
            text("nonce", login.nonce.expose()),
            text("verifier", login.verifier.expose()),
            ("secs".to_owned(), Value::from(lifetime_secs)),
        ];
        self.pool
            .run(
                "CREATE oidc_logins:$state = { nonce: $nonce, verifier: $verifier } \
                 EXPIRE time::from_unix(time::unix(time::now()) + $secs);",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn remove_login(&self, state: &str) -> Result<Option<LoginEntity>> {
        // One statement reads and removes: two callbacks with one state cannot both get the login. The loser of an
        // overlap is refused rather than answered, and it took nothing.
        match self
            .pool
            .run(
                "DELETE oidc_logins:$state RETURN BEFORE;",
                vec![text("state", state)],
            )
            .await
        {
            Ok(answers) => first_record(answers.into_iter().next())?
                .map(|value| LoginEntity::from_value(&value))
                .transpose(),
            Err(error) if lost_race(&error) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn get_link(&self, id: &str) -> Result<Option<LinkEntity>> {
        let answers = self
            .pool
            .run(
                "SELECT * FROM ONLY identity_links:$id;",
                vec![text("id", id)],
            )
            .await?;
        first_record(answers.into_iter().next())?
            .map(|value| LinkEntity::from_value(&value))
            .transpose()
    }

    async fn replace_link(
        &self,
        id: &str,
        user: &UserName,
        link: &LinkEntity,
    ) -> Result<LinkWritten> {
        let parameters = vec![
            text("id", id),
            text("user", user.as_str()),
            text("issuer", &link.issuer),
            text("subject", &link.subject),
        ];
        // The user's earlier link goes in the same transaction the new one is created in; `CREATE` refuses an id
        // that is there, so a link another user holds is never overwritten.
        let script = "BEGIN; \
             DELETE FROM identity_links WHERE user_name = $user LIMIT ALL; \
             CREATE identity_links:$id = { issuer: $issuer, subject: $subject, user_name: $user }; \
             COMMIT;";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(LinkWritten::Written),
            Err(error) if lost_race(&error) => Ok(LinkWritten::Lost),
            Err(error) => Err(error.into()),
        }
    }

    async fn links_of(&self, user: &UserName) -> Result<Vec<LinkEntity>> {
        let answers = self
            .pool
            .run(
                "SELECT * FROM identity_links WHERE user_name = $user USING INDEX by_link_user;",
                vec![text("user", user.as_str())],
            )
            .await?;
        all_records(answers.into_iter().next(), "identity link")?
            .iter()
            .map(LinkEntity::from_value)
            .collect()
    }

    async fn remove_link(&self, id: &str) -> Result<bool> {
        let answers = self
            .pool
            .run(
                "DELETE identity_links:$id RETURN BEFORE;",
                vec![text("id", id)],
            )
            .await?;
        Ok(first_record(answers.into_iter().next())?.is_some())
    }

    async fn put_logout(&self, user: &UserName, at: i64) -> Result<()> {
        self.pool
            .run(
                "UPSERT console_logouts:$user SET after = $at;",
                vec![
                    text("user", user.as_str()),
                    ("at".to_owned(), Value::from(at)),
                ],
            )
            .await?;
        Ok(())
    }

    async fn get_logout(&self, user: &UserName) -> Result<Option<i64>> {
        let answers = self
            .pool
            .run(
                "SELECT * FROM ONLY console_logouts:$user;",
                vec![text("user", user.as_str())],
            )
            .await?;
        first_record(answers.into_iter().next())?
            .map(|value| logout_after(&value))
            .transpose()
    }

    async fn insert_logout_id(&self, jti: &str, keep_secs: i64) -> Result<bool> {
        match self
            .pool
            .run(
                "CREATE oidc_logout_ids:$jti = { seen: time::unix(time::now()) } \
                 EXPIRE time::from_unix(time::unix(time::now()) + $secs);",
                vec![
                    text("jti", jti),
                    ("secs".to_owned(), Value::from(keep_secs)),
                ],
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if lost_race(&error) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}
