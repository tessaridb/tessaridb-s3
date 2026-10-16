//! The stored fields of the groups domain's records.

use tessari_s3_infrastructure::tessaridb::Value;

use crate::{Error, Result};

/// One group, stored as `groups:<space>/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEntity {
    /// The space it belongs to.
    pub space_name: String,
    /// Its name within that space.
    pub name: String,
    /// When it was created.
    pub created: (i64, u32),
}

/// One membership, stored as `group_members:<space>/<group>/<user>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberEntity {
    /// The group's id, `<space>/<name>`.
    pub group_id: String,
    /// The member.
    pub user_name: String,
}

/// One grant of a group, stored as `group_grants:<space>/<group>/<bucket>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupGrantEntity {
    /// The group's id, `<space>/<name>`.
    pub group_id: String,
    /// The bucket granted.
    pub bucket_name: String,
    /// Listing and reading.
    pub can_read: bool,
    /// Writing and deleting.
    pub can_write: bool,
}

fn field<'a>(value: &'a Value, record: &'static str, name: &'static str) -> Result<&'a Value> {
    match value {
        Value::Object(fields) => fields.get(name).ok_or(Error::Malformed {
            record,
            reason: name,
        }),
        _ => Err(Error::Malformed {
            record,
            reason: "not an object",
        }),
    }
}

fn text(value: &Value, record: &'static str, name: &'static str) -> Result<String> {
    match field(value, record, name)? {
        Value::String(text) => Ok(text.clone()),
        _ => Err(Error::Malformed {
            record,
            reason: name,
        }),
    }
}

fn flag(value: &Value, record: &'static str, name: &'static str) -> Result<bool> {
    match field(value, record, name)? {
        Value::Bool(flag) => Ok(*flag),
        _ => Err(Error::Malformed {
            record,
            reason: name,
        }),
    }
}

impl GroupEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        let created = match field(value, "group", "created")? {
            Value::Datetime { seconds, nanos } => (*seconds, *nanos),
            _ => {
                return Err(Error::Malformed {
                    record: "group",
                    reason: "created",
                });
            }
        };
        Ok(Self {
            space_name: text(value, "group", "space_name")?,
            name: text(value, "group", "name")?,
            created,
        })
    }
}

impl MemberEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        Ok(Self {
            group_id: text(value, "group member", "group_id")?,
            user_name: text(value, "group member", "user_name")?,
        })
    }
}

impl GroupGrantEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        Ok(Self {
            group_id: text(value, "group grant", "group_id")?,
            bucket_name: text(value, "group grant", "bucket_name")?,
            can_read: flag(value, "group grant", "can_read")?,
            can_write: flag(value, "group grant", "can_write")?,
        })
    }
}
