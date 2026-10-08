//! Actions in TessariDB: one message per action in the `console_actions` topic, which refuses `UPDATE` and `DELETE`
//! of a message, so the record is append-only by the store and not by this code's discipline. Every value is bound.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};

use super::entity::ActionEntity;
use super::model::NewAction;
use super::repository::ActionRepository;
use crate::answers::first_record;
use crate::{Error, Result};

/// The TessariDB action repository.
#[derive(Clone)]
pub struct TessariActions {
    pool: MetaPool,
}

impl TessariActions {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn text(value: &str) -> Value {
    Value::String(value.to_owned())
}

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "console_actions",
        reason,
    }
}

impl ActionRepository for TessariActions {
    async fn append(&self, action: &NewAction) -> Result<()> {
        let parameters = vec![
            ("operator".to_owned(), text(&action.operator)),
            ("operation".to_owned(), text(&action.operation)),
            ("target".to_owned(), text(&action.target)),
            (
                "reason".to_owned(),
                action.reason.as_deref().map_or(Value::None, text),
            ),
            ("outcome".to_owned(), text(&action.outcome)),
        ];
        // `at` is the store's clock, never the caller's.
        self.pool
            .run(
                "CREATE console_actions = { at: time::now(), operator: $operator, operation: $operation, \
                 target: $target, reason: $reason, outcome: $outcome };",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn last(&self) -> Result<u64> {
        let answers = self
            .pool
            .run("INFO FOR TOPIC console_actions;", Vec::new())
            .await?;
        let Some(Value::Object(fields)) = first_record(answers.into_iter().next())? else {
            return Err(malformed("INFO FOR TOPIC answered no object"));
        };
        match fields.get("last") {
            Some(Value::Number(Number::Integer(last))) => {
                u64::try_from(*last).map_err(|_| malformed("last"))
            }
            // An empty topic has no last position.
            None | Some(Value::None | Value::Null) => Ok(0),
            Some(_) => Err(malformed("last")),
        }
    }

    async fn after(&self, after: u64, limit: usize) -> Result<Vec<ActionEntity>> {
        let from = i64::try_from(after).map_err(|_| malformed("position"))?;
        let parameters = vec![("from".to_owned(), Value::Number(Number::Integer(from)))];
        // A position read starts at its position rather than walking the year of actions before it. `LIMIT` takes a
        // literal; the limit is this server's own integer and formats to digits only.
        let script = format!("READ FROM console_actions AFTER $from LIMIT {limit};");
        let answers = self.pool.run(&script, parameters).await?;
        match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => records
                .iter()
                .map(|(_, value)| ActionEntity::from_value(value))
                .collect(),
            _ => Err(malformed("a topic read answered no records")),
        }
    }
}
