//! Usage in TessariDB: one grouped read over `objects` measures every bucket, the result replaces the one
//! `bucket_usage:1` record, and the `usage_claims` space holds the expiring claim of the member measuring now. Every
//! value is bound.

use tessari_s3_constants::{DATA_BLOCK_SIZE, USAGE_CLAIM_SECS};
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};

use super::entity::{BucketUsageEntity, UsageEntity};
use super::occupancy::{Measured, ObjectFold, PartFold};
use super::repository::UsageRepository;
use crate::answers::{all_records, first_record};
use crate::{Error, Result};

/// The TessariDB usage repository.
#[derive(Clone)]
pub struct TessariUsage {
    pool: MetaPool,
}

impl TessariUsage {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket_usage",
        reason,
    }
}

impl UsageRepository for TessariUsage {
    async fn claim(&self, holder: &str) -> Result<bool> {
        let parameters = vec![
            ("holder".to_owned(), Value::String(holder.to_owned())),
            ("pass".to_owned(), Value::String("usage".to_owned())),
        ];
        // Free, or already this member's. `EXPIRE` takes the duration as written; it is this server's own constant,
        // formatted to digits only.
        let script = format!(
            "SET usage_claims:$pass = $holder IF ABSENT EXPIRE {USAGE_CLAIM_SECS}s; \
             SET usage_claims:$pass = $holder IF = $holder EXPIRE {USAGE_CLAIM_SECS}s;"
        );
        let mut taken = false;
        for answer in self.pool.run(&script, parameters).await? {
            match answer {
                Answer::Value {
                    value: Value::Bool(wrote),
                    ..
                } => taken = taken || wrote,
                _ => return Err(malformed("a conditional set answered no boolean")),
            }
        }
        Ok(taken)
    }

    async fn measure(&self) -> Result<Measured> {
        // Whole-table folds: they read every object and part record by design, once per pass, never per request.
        // A data file's stripes are `ceil(size / stripe)`; an inline object has no data file.
        const SCRIPT: &str = "\
            SELECT bucket_name, count(*) AS objects, sum(size) AS bytes, \
             sum(IF inline != NONE THEN size ELSE 0 END) AS inline_bytes, \
             sum(IF data != NONE THEN size ELSE 0 END) AS data_bytes, \
             sum(IF data != NONE THEN math::ceil(size / $stripe) ELSE 0 END) AS data_stripes \
             FROM objects GROUP BY bucket_name; \
            SELECT upload, sum(size) AS bytes, sum(math::ceil(size / $stripe)) AS stripes FROM parts GROUP BY upload; \
            SELECT upload, bucket_name FROM objects WHERE upload != NONE; \
            SELECT upload, bucket_name FROM pending;";
        let parameters = vec![(
            "stripe".to_owned(),
            Value::Number(Number::Integer(i64::from(DATA_BLOCK_SIZE))),
        )];
        let mut answers = self.pool.run(SCRIPT, parameters).await?.into_iter();
        let mut next = || all_records(answers.next(), "bucket_usage");
        let objects = next()?
            .iter()
            .map(object_fold)
            .collect::<Result<Vec<_>>>()?;
        let parts = next()?.iter().map(part_fold).collect::<Result<Vec<_>>>()?;
        let mut owners = std::collections::BTreeMap::new();
        for owner in next()?.iter().chain(next()?.iter()) {
            let (upload, bucket) = owner_of(owner)?;
            owners.insert(upload, bucket);
        }
        Ok(Measured {
            objects,
            parts,
            owners,
        })
    }

    async fn store(&self, buckets: &[BucketUsageEntity]) -> Result<()> {
        let parameters = vec![(
            "buckets".to_owned(),
            Value::Array(buckets.iter().map(BucketUsageEntity::to_value).collect()),
        )];
        // The whole record, deliberately: a bucket emptied since the last pass must disappear from it.
        self.pool
            .run(
                "UPSERT bucket_usage:1 = { taken: time::now(), buckets: $buckets };",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn latest(&self) -> Result<Option<UsageEntity>> {
        let answers = self
            .pool
            .run("SELECT * FROM ONLY bucket_usage:1;", Vec::new())
            .await?;
        first_record(answers.into_iter().next())?
            .as_ref()
            .map(UsageEntity::from_value)
            .transpose()
    }
}

/// A non-negative whole count, as a sum answers it: an integer, or a decimal with no fraction (a sum over a division).
fn count(fields: &std::collections::BTreeMap<String, Value>, field: &'static str) -> Result<u64> {
    let whole = match fields.get(field) {
        Some(Value::Number(Number::Integer(number))) => i128::from(*number),
        Some(Value::Number(Number::Decimal { mantissa, scale })) => {
            let unit = 10_i128
                .checked_pow(*scale)
                .ok_or_else(|| malformed(field))?;
            if mantissa.checked_rem(unit) != Some(0) {
                return Err(malformed(field));
            }
            mantissa.checked_div(unit).ok_or_else(|| malformed(field))?
        }
        _ => return Err(malformed(field)),
    };
    u64::try_from(whole).map_err(|_| malformed(field))
}

fn fields(value: &Value) -> Result<&std::collections::BTreeMap<String, Value>> {
    match value {
        Value::Object(fields) => Ok(fields),
        _ => Err(malformed("not an object")),
    }
}

fn text(fields: &std::collections::BTreeMap<String, Value>, field: &'static str) -> Result<String> {
    match fields.get(field) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(field)),
    }
}

fn upload(fields: &std::collections::BTreeMap<String, Value>) -> Result<[u8; 16]> {
    match fields.get("upload") {
        Some(Value::Uuid(id)) => Ok(*id),
        _ => Err(malformed("upload")),
    }
}

fn object_fold(value: &Value) -> Result<ObjectFold> {
    let row = fields(value)?;
    Ok(ObjectFold {
        bucket: text(row, "bucket_name")?,
        objects: count(row, "objects")?,
        bytes: count(row, "bytes")?,
        inline_bytes: count(row, "inline_bytes")?,
        data_bytes: count(row, "data_bytes")?,
        data_stripes: count(row, "data_stripes")?,
    })
}

fn part_fold(value: &Value) -> Result<PartFold> {
    let row = fields(value)?;
    Ok(PartFold {
        upload: upload(row)?,
        bytes: count(row, "bytes")?,
        stripes: count(row, "stripes")?,
    })
}

fn owner_of(value: &Value) -> Result<([u8; 16], String)> {
    let row = fields(value)?;
    Ok((upload(row)?, text(row, "bucket_name")?))
}
