//! The `users`, `access_keys` and `grants` records exactly as stored.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::Value;

use crate::{Error, Result};

/// The stored fields of one user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserEntity {
    /// The user's name, also its record id.
    pub name: String,
    /// The space it belongs to.
    pub space_name: String,
    /// `admin` or `member`.
    pub space_role: String,
    /// Whether a member may create buckets in the space.
    pub create_buckets: bool,
    /// Store-wide operator.
    pub operator: bool,
    /// Store-wide cluster viewer.
    pub cluster_viewer: bool,
    /// A disabled user's keys resolve to nothing.
    pub disabled: bool,
    /// Creation time: seconds and nanoseconds since the epoch.
    pub created: (i64, u32),
}

/// The stored fields of one access key; its id is the record's id and is not repeated here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessKeyEntity {
    /// The user holding the key.
    pub user_name: String,
    /// The sealed secret: ciphertext followed by its tag.
    pub secret: Vec<u8>,
    /// The nonce it was sealed under.
    pub nonce: Vec<u8>,
    /// Which root key sealed it.
    pub kek_id: String,
    /// Which algorithm sealed it.
    pub algorithm: String,
    /// A disabled key resolves to nothing.
    pub disabled: bool,
    /// Creation time: seconds and nanoseconds since the epoch.
    pub created: (i64, u32),
}

/// The stored fields of one grant: a user's access to one bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantEntity {
    /// The user granted.
    pub user_name: String,
    /// The bucket.
    pub bucket_name: String,
    /// Listing and reading objects.
    pub can_read: bool,
    /// Writing and deleting objects.
    pub can_write: bool,
}

/// A record's fields, read by name; every miss names the record kind and the field.
struct Fields<'a> {
    record: &'static str,
    fields: &'a BTreeMap<String, Value>,
}

impl<'a> Fields<'a> {
    fn of(value: &'a Value, record: &'static str) -> Result<Self> {
        match value {
            Value::Object(fields) => Ok(Self { record, fields }),
            _ => Err(Error::Malformed {
                record,
                reason: "not an object",
            }),
        }
    }

    const fn malformed(&self, field: &'static str) -> Error {
        Error::Malformed {
            record: self.record,
            reason: field,
        }
    }

    fn text(&self, field: &'static str) -> Result<String> {
        match self.fields.get(field) {
            Some(Value::String(text)) => Ok(text.clone()),
            _ => Err(self.malformed(field)),
        }
    }

    fn flag(&self, field: &'static str) -> Result<bool> {
        match self.fields.get(field) {
            Some(Value::Bool(flag)) => Ok(*flag),
            _ => Err(self.malformed(field)),
        }
    }

    fn bytes(&self, field: &'static str) -> Result<Vec<u8>> {
        match self.fields.get(field) {
            Some(Value::Bytes(bytes)) => Ok(bytes.clone()),
            _ => Err(self.malformed(field)),
        }
    }

    fn instant(&self, field: &'static str) -> Result<(i64, u32)> {
        match self.fields.get(field) {
            Some(Value::Datetime { seconds, nanos }) => Ok((*seconds, *nanos)),
            _ => Err(self.malformed(field)),
        }
    }
}

impl UserEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = Fields::of(value, "user")?;
        Ok(Self {
            name: fields.text("name")?,
            space_name: fields.text("space_name")?,
            space_role: fields.text("space_role")?,
            create_buckets: fields.flag("create_buckets")?,
            operator: fields.flag("operator")?,
            cluster_viewer: fields.flag("cluster_viewer")?,
            disabled: fields.flag("disabled")?,
            created: fields.instant("created")?,
        })
    }
}

impl AccessKeyEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = Fields::of(value, "access key")?;
        Ok(Self {
            user_name: fields.text("user_name")?,
            secret: fields.bytes("secret")?,
            nonce: fields.bytes("nonce")?,
            kek_id: fields.text("kek_id")?,
            algorithm: fields.text("algorithm")?,
            disabled: fields.flag("disabled")?,
            created: fields.instant("created")?,
        })
    }
}

impl GrantEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        let fields = Fields::of(value, "grant")?;
        Ok(Self {
            user_name: fields.text("user_name")?,
            bucket_name: fields.text("bucket_name")?,
            can_read: fields.flag("can_read")?,
            can_write: fields.flag("can_write")?,
        })
    }
}
