//! Membership in TessariDB: `s3_members:<node id>` per node, never deleted (a removed node stays as a tombstone);
//! every value is bound, and a `WHERE` on an edit is a compare-and-set, so a refused condition writes nothing.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Number, RefusalClass, Value};

use super::entity::MemberEntity;
use super::repository::{Grant, MemberRepository};
use crate::Result;
use crate::answers::{all_records, first_record};

/// The TessariDB membership repository.
#[derive(Clone)]
pub struct TessariMembers {
    pool: MetaPool,
}

impl TessariMembers {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn string(name: &str, value: &str) -> (String, Value) {
    (name.to_owned(), Value::String(value.to_owned()))
}

fn optional(name: &str, value: Option<&str>) -> (String, Value) {
    (
        name.to_owned(),
        value.map_or(Value::None, |value| Value::String(value.to_owned())),
    )
}

/// A token's lifetime as a bound integer; the model bounds it to a day, so it always fits.
fn lifetime(secs: u64) -> Value {
    Value::Number(Number::Integer(i64::try_from(secs).unwrap_or(i64::MAX)))
}

/// The fields a grant writes; `{expires}` is `NONE` or the token's lapse, so `$ttl` is bound only when used.
fn fields(token: bool) -> String {
    let expires = if token {
        "time::now() + duration::from_secs($ttl)"
    } else {
        "NONE"
    };
    format!(
        "state: 'approved', fingerprint: $fingerprint, token_digest: $digest, token_expires: {expires}, \
         reason: $reason, actor: $by, at: time::now()"
    )
}

impl MemberRepository for TessariMembers {
    async fn get(&self, node: &str) -> Result<Option<MemberEntity>> {
        let answers = self
            .pool
            .run(
                "SELECT * FROM ONLY s3_members:$node;",
                vec![string("node", node)],
            )
            .await?;
        first_record(answers.into_iter().next())?
            .map(|value| MemberEntity::from_value(&value))
            .transpose()
    }

    async fn list(&self) -> Result<Vec<MemberEntity>> {
        // 256: the most nodes this server reads a cluster as.
        let answers = self
            .pool
            .run(
                "SELECT * FROM s3_members ORDER BY node LIMIT 256;",
                Vec::new(),
            )
            .await?;
        all_records(answers.into_iter().next(), "member")?
            .iter()
            .map(MemberEntity::from_value)
            .collect()
    }

    async fn create(&self, grant: &Grant<'_>) -> Result<bool> {
        let (parameters, fields) = grant_parts(grant);
        let create = format!("CREATE s3_members:$node = {{ node: $node, {fields} }};");
        match self.pool.run(&create, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn grant(&self, grant: &Grant<'_>) -> Result<bool> {
        if self.create(grant).await? {
            return Ok(true);
        }
        let (parameters, fields) = grant_parts(grant);
        // It exists, and records are never deleted: replace it unless it is a tombstone.
        let update =
            format!("UPDATE s3_members:$node MERGE {{ {fields} }} WHERE state = 'approved';");
        match self.pool.run(&update, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn bind(&self, node: &str, digest: &str, fingerprint: &str) -> Result<bool> {
        let script = "UPDATE s3_members:$node SET fingerprint = $fingerprint, token_digest = NONE, \
             token_expires = NONE WHERE state = 'approved' AND token_digest = $digest \
             AND token_expires > time::now();";
        let parameters = vec![
            string("node", node),
            string("digest", digest),
            string("fingerprint", fingerprint),
        ];
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn remove(&self, node: &str, by: &str, reason: &str) -> Result<bool> {
        let script = "BEGIN; UPDATE s3_members:$node SET state = 'removed', fingerprint = NONE, \
             token_digest = NONE, token_expires = NONE, reason = $reason, actor = $by, at = time::now() \
             WHERE state = 'approved'; DELETE s3_nodes:$node; COMMIT;";
        let parameters = vec![
            string("node", node),
            string("by", by),
            string("reason", reason),
        ];
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}

/// The parameters `grant` binds and the fields it writes.
fn grant_parts(grant: &Grant<'_>) -> (Vec<(String, Value)>, String) {
    let mut parameters = vec![
        string("node", grant.node),
        optional("fingerprint", grant.fingerprint),
        optional("digest", grant.token.map(|(digest, _)| digest)),
        string("reason", grant.reason),
        string("by", grant.by),
    ];
    if let Some((_, secs)) = grant.token {
        parameters.push(("ttl".to_owned(), lifetime(secs)));
    }
    (parameters, fields(grant.token.is_some()))
}
