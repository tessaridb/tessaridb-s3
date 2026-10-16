//! A bucket's default encryption as its record holds it: `encryption` is present while the bucket has one and carries
//! the `algorithm` (`AES256` or `aws:kms`), the named `key` under `aws:kms`, and whether the request asked for a
//! `bucket_key` — recorded and answered back, since each object's own data key is what seals it here — and whether
//! writes may bring their own key, `allow_customer_keys`; a default stored before that existed keeps them blocked.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::Value;

use super::model::BucketEncryption;
use crate::encryption::{KeyName, Sealing};
use crate::{Error, Result};

const SERVER: &str = "AES256";
const NAMED: &str = "aws:kms";

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket",
        reason,
    }
}

/// The stored form of `encryption`.
pub(crate) fn to_value(encryption: &BucketEncryption) -> Value {
    let mut fields = BTreeMap::from([
        ("bucket_key".to_owned(), Value::Bool(encryption.bucket_key)),
        (
            "allow_customer_keys".to_owned(),
            Value::Bool(encryption.allow_customer_keys),
        ),
    ]);
    match &encryption.sealing {
        Sealing::Server => {
            fields.insert("algorithm".to_owned(), Value::String(SERVER.to_owned()));
        }
        Sealing::Named(name) => {
            fields.insert("algorithm".to_owned(), Value::String(NAMED.to_owned()));
            fields.insert("key".to_owned(), Value::String(name.as_str().to_owned()));
        }
    }
    Value::Object(fields)
}

/// The default encryption a stored `encryption` field holds.
pub(crate) fn from_value(value: &Value) -> Result<BucketEncryption> {
    let Value::Object(fields) = value else {
        return Err(malformed("encryption"));
    };
    let sealing = match (fields.get("algorithm"), fields.get("key")) {
        (Some(Value::String(algorithm)), None) if algorithm == SERVER => Sealing::Server,
        (Some(Value::String(algorithm)), Some(Value::String(key))) if algorithm == NAMED => {
            Sealing::Named(KeyName::new(key).ok_or(malformed("an encryption key name"))?)
        }
        _ => return Err(malformed("an encryption algorithm")),
    };
    let bucket_key = match fields.get("bucket_key") {
        Some(Value::Bool(enabled)) => *enabled,
        _ => return Err(malformed("an encryption bucket_key")),
    };
    let allow_customer_keys = match fields.get("allow_customer_keys") {
        None => false,
        Some(Value::Bool(allowed)) => *allowed,
        Some(_) => return Err(malformed("an encryption allow_customer_keys")),
    };
    Ok(BucketEncryption {
        sealing,
        bucket_key,
        allow_customer_keys,
    })
}

#[cfg(test)]
mod tests {
    use tessari_s3_infrastructure::tessaridb::Value;

    use super::{from_value, to_value};
    use crate::buckets::BucketEncryption;
    use crate::encryption::{KeyName, Sealing};

    #[test]
    fn a_default_encryption_round_trips_through_its_stored_form() {
        let named = Sealing::Named(KeyName::new("logs-key").expect("usable"));
        for (sealing, bucket_key, allow_customer_keys) in
            [(Sealing::Server, false, false), (named, true, true)]
        {
            let encryption = BucketEncryption {
                sealing,
                bucket_key,
                allow_customer_keys,
            };
            assert_eq!(
                from_value(&to_value(&encryption)).expect("reads"),
                encryption
            );
        }
    }

    #[test]
    fn a_stored_form_that_names_no_known_algorithm_is_refused() {
        let Value::Object(mut fields) = to_value(&BucketEncryption {
            sealing: Sealing::Server,
            bucket_key: false,
            allow_customer_keys: false,
        }) else {
            panic!("an object");
        };
        fields.insert(
            "algorithm".to_owned(),
            Value::String("aws:kms:dsse".to_owned()),
        );
        assert!(from_value(&Value::Object(fields)).is_err());
    }

    #[test]
    fn a_default_stored_before_customer_keys_existed_keeps_them_blocked() {
        let Value::Object(mut fields) = to_value(&BucketEncryption {
            sealing: Sealing::Server,
            bucket_key: false,
            allow_customer_keys: true,
        }) else {
            panic!("an object");
        };
        assert_eq!(fields.get("allow_customer_keys"), Some(&Value::Bool(true)));
        fields.remove("allow_customer_keys");
        assert!(
            !from_value(&Value::Object(fields))
                .expect("reads")
                .allow_customer_keys
        );
    }
}
