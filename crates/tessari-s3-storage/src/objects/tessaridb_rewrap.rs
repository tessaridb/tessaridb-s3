//! The statements a re-wrap pass needs: the records whose data key is wrapped under a given key, how many there are,
//! and the compare-and-set that puts a re-wrapped key in place only while the record still holds the one it read.

use tessari_s3_infrastructure::tessaridb::{Number, RefusalClass, Value};

use super::super::entity::{Fields, sealed, sealed_fields};
use super::super::version_id::{key_prefix, record_id};
use crate::answers::{all_records, first_record};
use crate::encryption::WrappedKey;
use crate::{Error, Result};

/// The tables whose records carry a wrapped data key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holder {
    Versions,
    Objects,
    Parts,
}

impl Holder {
    pub(crate) const ALL: [Self; 3] = [Self::Versions, Self::Objects, Self::Parts];

    const fn table(self) -> &'static str {
        match self {
            Self::Versions => "versions",
            Self::Objects => "objects",
            Self::Parts => "parts",
        }
    }

    /// The fields that rebuild a record's address, beside its data id and its wrapped key.
    const fn address_fields(self) -> &'static str {
        match self {
            Self::Versions => "bucket_name, key, seq",
            Self::Objects => "bucket_name, key",
            Self::Parts => "upload, number",
        }
    }
}

/// A record whose data key is wrapped under an older key.
#[derive(Debug)]
pub(crate) struct Behind {
    pub(crate) holder: Holder,
    /// The record's id, as its table is addressed by.
    pub(crate) address: Value,
    /// The data file whose key it wraps; the wrap is bound to it.
    pub(crate) data: [u8; 16],
    pub(crate) wrapped: WrappedKey,
}

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "a sealed record",
        reason,
    }
}

fn text<'a>(fields: &'a Fields, name: &'static str) -> Result<&'a str> {
    match fields.get(name) {
        Some(Value::String(text)) => Ok(text),
        _ => Err(malformed(name)),
    }
}

fn unsigned(fields: &Fields, name: &'static str) -> Result<u64> {
    match fields.get(name) {
        Some(Value::Number(Number::Integer(number))) => {
            u64::try_from(*number).map_err(|_| malformed(name))
        }
        _ => Err(malformed(name)),
    }
}

fn uuid(fields: &Fields, name: &'static str) -> Result<[u8; 16]> {
    match fields.get(name) {
        Some(Value::Uuid(bytes)) => Ok(*bytes),
        _ => Err(malformed(name)),
    }
}

/// The address of `holder`'s record whose fields are `fields`.
fn address(holder: Holder, fields: &Fields) -> Result<Value> {
    Ok(match holder {
        Holder::Versions => Value::Bytes(record_id(
            &key_prefix(text(fields, "bucket_name")?, text(fields, "key")?),
            unsigned(fields, "seq")?,
        )),
        Holder::Objects => Value::String(format!(
            "{}/{}",
            text(fields, "bucket_name")?,
            text(fields, "key")?
        )),
        Holder::Parts => {
            let upload: String = uuid(fields, "upload")?
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            Value::String(format!("{upload}/{:05}", unsigned(fields, "number")?))
        }
    })
}

impl super::TessariObjects {
    /// Up to `limit` of `holder`'s records whose data key is wrapped under `kek_id`.
    pub(crate) async fn wrapped_under(
        &self,
        holder: Holder,
        kek_id: &str,
        limit: usize,
    ) -> Result<Vec<Behind>> {
        let script = format!(
            "SELECT {}, data, sse_alg, sse_kek, sse_nonce, sse_key FROM {} WHERE sse_kek = $kek LIMIT {limit};",
            holder.address_fields(),
            holder.table()
        );
        let answers = self
            .pool
            .run(
                &script,
                vec![("kek".to_owned(), Value::String(kek_id.to_owned()))],
            )
            .await?;
        all_records(answers.into_iter().next(), "a sealed record")?
            .into_iter()
            .map(|record| {
                let Value::Object(fields) = record else {
                    return Err(malformed("not an object"));
                };
                Ok(Behind {
                    holder,
                    address: address(holder, &fields)?,
                    data: uuid(&fields, "data")?,
                    wrapped: sealed(&fields)?.ok_or_else(|| malformed("no wrapped key"))?,
                })
            })
            .collect()
    }

    /// How many of `holder`'s records have their data key wrapped under `kek_id`.
    pub(crate) async fn count_wrapped_under(&self, holder: Holder, kek_id: &str) -> Result<u64> {
        let script = format!(
            "SELECT count(*) AS n FROM {} WHERE sse_kek = $kek;",
            holder.table()
        );
        let answers = self
            .pool
            .run(
                &script,
                vec![("kek".to_owned(), Value::String(kek_id.to_owned()))],
            )
            .await?;
        match first_record(answers.into_iter().next())? {
            None => Ok(0),
            Some(Value::Object(fields)) => unsigned(&fields, "n"),
            Some(_) => Err(malformed("a count")),
        }
    }

    /// Puts `fresh` in place of `behind`'s wrapped key, only while the record still holds the key `behind` read;
    /// answers whether it did.
    pub(crate) async fn rewrap(&self, behind: &Behind, fresh: &WrappedKey) -> Result<bool> {
        let script = format!(
            "UPDATE {}:$at SET sse_alg = $sse_alg, sse_kek = $sse_kek, sse_nonce = $sse_nonce, sse_key = $sse_key \
             WHERE sse_kek = $was_kek AND sse_key = $was_key RETURN AFTER;",
            behind.holder.table()
        );
        let mut parameters: Vec<(String, Value)> = sealed_fields(fresh)
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect();
        parameters.extend([
            ("at".to_owned(), behind.address.clone()),
            (
                "was_kek".to_owned(),
                Value::String(behind.wrapped.kek_id.clone()),
            ),
            (
                "was_key".to_owned(),
                Value::Bytes(behind.wrapped.ciphertext.clone()),
            ),
        ]);
        match self.pool.run(&script, parameters).await {
            Ok(answers) => Ok(first_record(answers.into_iter().next())?.is_some()),
            // The record is gone, or no longer holds the key that was read: another pass or a write got there first.
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}
