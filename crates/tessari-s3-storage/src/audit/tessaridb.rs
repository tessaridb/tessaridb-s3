//! The access log in TessariDB: one message per request in the `s3_access_log` topic, under the request id. A topic
//! refuses `UPDATE` and `DELETE` of a message, so the log is append-only by the store. Every value is bound.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Number, RefusalClass, Value};

use super::model::AccessRecord;
use super::repository::AuditRepository;
use crate::Result;

/// The TessariDB access-log repository.
#[derive(Clone)]
pub struct TessariAudit {
    pool: MetaPool,
}

impl TessariAudit {
    /// A repository over the region's `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn text(value: &str) -> Value {
    Value::String(value.to_owned())
}

fn optional(value: Option<&str>) -> Value {
    value.map_or(Value::None, text)
}

fn integer(value: u64) -> Value {
    Value::Number(Number::Integer(i64::try_from(value).unwrap_or(i64::MAX)))
}

/// The statement appending the record whose parameters end in `suffix`.
fn create(suffix: usize) -> String {
    format!(
        "CREATE s3_access_log:$id{suffix} = {{ at: $at{suffix}, method: $method{suffix}, \
         operation: $operation{suffix}, bucket: $bucket{suffix}, key: $key{suffix}, \
         principal: $principal{suffix}, status: $status{suffix}, code: $code{suffix}, bytes: $bytes{suffix}, \
         millis: $millis{suffix}, client: $client{suffix} }};"
    )
}

/// `record`'s parameters, each name ending in `suffix`.
fn parameters(record: &AccessRecord, suffix: usize) -> [(String, Value); 12] {
    [
        ("id", text(&record.request_id)),
        (
            "at",
            Value::Datetime {
                seconds: record.at_secs,
                nanos: 0,
            },
        ),
        ("method", text(&record.method)),
        ("operation", text(record.operation)),
        ("bucket", optional(record.bucket.as_deref())),
        ("key", optional(record.key.as_deref())),
        ("principal", text(&record.principal)),
        ("status", integer(u64::from(record.status))),
        ("code", text(record.code)),
        ("bytes", integer(record.bytes)),
        ("millis", integer(record.millis)),
        ("client", optional(record.client.as_deref())),
    ]
    .map(|(name, value)| (format!("{name}{suffix}"), value))
}

impl AuditRepository for TessariAudit {
    async fn append_all(&self, records: &[AccessRecord]) -> Result<()> {
        let mut script = String::from("BEGIN;");
        let mut bound = Vec::with_capacity(records.len().saturating_mul(12));
        for (at, record) in records.iter().enumerate() {
            script.push_str(&create(at));
            bound.extend(parameters(record, at));
        }
        script.push_str("COMMIT;");
        self.pool.run(&script, bound).await?;
        Ok(())
    }

    async fn append_one(&self, record: &AccessRecord) -> Result<bool> {
        match self
            .pool
            .run(&create(0), parameters(record, 0).into())
            .await
        {
            Ok(_) => Ok(true),
            // `RecordExists`: this request was logged by an earlier attempt.
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}
