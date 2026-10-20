//! A `buckets` record exactly as stored.

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

/// The stored fields of one bucket.
#[derive(Debug, Clone, PartialEq)]
pub struct BucketEntity {
    /// The bucket's name, held as a field so it is read back typed rather than parsed out of a record id.
    pub name: String,
    /// Creation time: seconds and nanoseconds since the epoch.
    pub created: (i64, u32),
    /// The region it was created in.
    pub region: String,
    /// A fresh uuid per creation, so a bucket deleted and created again is not the same bucket.
    pub incarnation: [u8; 16],
    /// The space that owns the bucket.
    pub space_name: String,
    /// The user who created it; absent for the root credential's buckets.
    pub creator: Option<String>,
    /// The byte limit an operator set; absent for none.
    pub max_bytes: Option<i64>,
    /// The object limit an operator set; absent for none.
    pub max_objects: Option<i64>,
    /// `'Enabled'` or `'Suspended'`; absent while the bucket was never versioned.
    pub versioning: Option<String>,
    /// The lifecycle rules as stored; absent while none are set.
    pub lifecycle: Option<Vec<Value>>,
    /// The CORS rules as stored; absent while none are set.
    pub cors: Option<Vec<Value>>,
    /// The notification configuration as stored; absent while none is set.
    pub notify: Option<Value>,
    /// The replication configuration as stored; absent while none is set.
    pub replication: Option<Value>,
    /// The Object Lock configuration as stored; absent while the lock is off.
    pub object_lock: Option<Value>,
    /// The media rules as stored; absent while none are set.
    pub media: Option<Vec<Value>>,
    /// How many times the media rules were set; 0 before the first.
    pub media_version: u64,
    /// The tags as stored; absent while none are set.
    pub tags: Option<Value>,
    /// The default encryption as stored; absent while the bucket has none.
    pub encryption: Option<Value>,
    /// The bucket policy's JSON text; absent while none is set.
    pub policy: Option<String>,
    /// The Block Public Access switches as four `0`/`1` flags, empty once removed; absent while never set.
    pub public_access: Option<String>,
    /// `local` when the bucket's writes are acknowledged by its region's majority alone; absent otherwise.
    pub acknowledge: Option<String>,
}

impl BucketEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        let malformed = |reason| Error::Malformed {
            record: "bucket",
            reason,
        };
        let Value::Object(fields) = value else {
            return Err(malformed("not an object"));
        };
        let name = match fields.get("name") {
            Some(Value::String(name)) => name.clone(),
            _ => return Err(malformed("name")),
        };
        let created = match fields.get("created") {
            Some(Value::Datetime { seconds, nanos }) => (*seconds, *nanos),
            _ => return Err(malformed("created")),
        };
        let region = match fields.get("region") {
            Some(Value::String(region)) => region.clone(),
            _ => return Err(malformed("region")),
        };
        let incarnation = match fields.get("incarnation") {
            Some(Value::Uuid(bytes)) => *bytes,
            _ => return Err(malformed("incarnation")),
        };
        let space_name = match fields.get("space_name") {
            Some(Value::String(space)) => space.clone(),
            _ => return Err(malformed("space_name")),
        };
        let creator = match fields.get("creator") {
            None | Some(Value::None) => None,
            Some(Value::String(creator)) => Some(creator.clone()),
            Some(_) => return Err(malformed("creator")),
        };
        let versioning = match fields.get("versioning") {
            None | Some(Value::None) => None,
            Some(Value::String(state)) => Some(state.clone()),
            Some(_) => return Err(malformed("versioning")),
        };
        let lifecycle = match fields.get("lifecycle") {
            None | Some(Value::None) => None,
            Some(Value::Array(rules)) => Some(rules.clone()),
            Some(_) => return Err(malformed("lifecycle")),
        };
        let cors = match fields.get("cors") {
            None | Some(Value::None) => None,
            Some(Value::Array(rules)) => Some(rules.clone()),
            Some(_) => return Err(malformed("cors")),
        };
        let replication = match fields.get("replication") {
            None | Some(Value::None) => None,
            Some(configuration @ Value::Object(_)) => Some(configuration.clone()),
            Some(_) => return Err(malformed("replication")),
        };
        let object_lock = match fields.get("object_lock") {
            None | Some(Value::None) => None,
            Some(configuration @ Value::Object(_)) => Some(configuration.clone()),
            Some(_) => return Err(malformed("object_lock")),
        };
        let tags = match fields.get("tags") {
            None | Some(Value::None) => None,
            Some(tags @ Value::Object(_)) => Some(tags.clone()),
            Some(_) => return Err(malformed("tags")),
        };
        let encryption = match fields.get("encryption") {
            None | Some(Value::None) => None,
            Some(configuration @ Value::Object(_)) => Some(configuration.clone()),
            Some(_) => return Err(malformed("encryption")),
        };
        let notify = match fields.get("notify") {
            None | Some(Value::None) => None,
            Some(configuration @ Value::Object(_)) => Some(configuration.clone()),
            Some(_) => return Err(malformed("notify")),
        };
        let media_version = match fields.get("media_version") {
            Some(Value::Number(Number::Integer(times))) => {
                u64::try_from(*times).map_err(|_| malformed("media_version"))?
            }
            None | Some(Value::None | Value::Null) => 0,
            Some(_) => return Err(malformed("media_version")),
        };
        let media = match fields.get("media") {
            None | Some(Value::None) => None,
            Some(Value::Array(rules)) => Some(rules.clone()),
            Some(_) => return Err(malformed("media")),
        };
        let policy = match fields.get("policy") {
            None | Some(Value::None) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => return Err(malformed("policy")),
        };
        let public_access = match fields.get("public_access") {
            None | Some(Value::None) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => return Err(malformed("public_access")),
        };
        let acknowledge = match fields.get("acknowledge") {
            None | Some(Value::None) => None,
            Some(Value::String(level)) => Some(level.clone()),
            Some(_) => return Err(malformed("acknowledge")),
        };
        let limit = |field: &'static str| match fields.get(field) {
            None | Some(Value::None) => Ok(None),
            Some(Value::Number(Number::Integer(limit))) => Ok(Some(*limit)),
            Some(_) => Err(malformed(field)),
        };
        Ok(Self {
            name,
            created,
            region,
            incarnation,
            space_name,
            creator,
            max_bytes: limit("max_bytes")?,
            max_objects: limit("max_objects")?,
            versioning,
            lifecycle,
            cors,
            notify,
            replication,
            object_lock,
            media,
            media_version,
            tags,
            encryption,
            policy,
            public_access,
            acknowledge,
        })
    }
}
