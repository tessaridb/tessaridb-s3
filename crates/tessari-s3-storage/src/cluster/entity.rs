//! `s3_nodes`, `layouts` and `layout_current` records exactly as stored.

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

/// One registered node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeEntity {
    /// The node id.
    pub(crate) node: String,
    /// `host:port` of its internal surface.
    pub(crate) endpoint: String,
    /// Its failure domain, when it names one.
    pub(crate) zone: Option<String>,
}

/// One node of a layout's set, with the zone it was in when the layout was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberEntity {
    /// The node id.
    pub(crate) node: String,
    /// Its zone, when it named one.
    pub(crate) zone: Option<String>,
}

/// One layout version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutEntity {
    /// The version.
    pub(crate) version: i64,
    /// k.
    pub(crate) data: i64,
    /// m.
    pub(crate) parity: i64,
    /// The erasure sets, each one node per shard.
    pub(crate) sets: Vec<Vec<MemberEntity>>,
}

fn malformed(record: &'static str, reason: &'static str) -> Error {
    Error::Malformed { record, reason }
}

fn fields<'v>(
    value: &'v Value,
    record: &'static str,
) -> Result<&'v std::collections::BTreeMap<String, Value>> {
    match value {
        Value::Object(fields) => Ok(fields),
        _ => Err(malformed(record, "not an object")),
    }
}

fn text(value: Option<&Value>, record: &'static str, field: &'static str) -> Result<String> {
    match value {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(record, field)),
    }
}

/// An optional text field: absent (`NONE`) is `None`, anything but text is malformed.
fn optional_text(
    value: Option<&Value>,
    record: &'static str,
    field: &'static str,
) -> Result<Option<String>> {
    match value {
        None | Some(Value::None) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(malformed(record, field)),
    }
}

fn integer(value: Option<&Value>, field: &'static str) -> Result<i64> {
    match value {
        Some(Value::Number(Number::Integer(number))) => Ok(*number),
        _ => Err(malformed("layout", field)),
    }
}

impl NodeEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`].
    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        let fields = fields(value, "node")?;
        Ok(Self {
            node: text(fields.get("node"), "node", "node")?,
            endpoint: text(fields.get("endpoint"), "node", "endpoint")?,
            zone: optional_text(fields.get("zone"), "node", "zone")?,
        })
    }
}

impl LayoutEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`].
    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        let fields = fields(value, "layout")?;
        let sets = match fields.get("sets") {
            Some(Value::Array(sets)) => sets
                .iter()
                .map(|set| match set {
                    Value::Array(members) => members.iter().map(MemberEntity::from_value).collect(),
                    _ => Err(malformed("layout", "sets")),
                })
                .collect::<Result<Vec<_>>>()?,
            _ => return Err(malformed("layout", "sets")),
        };
        Ok(Self {
            version: integer(fields.get("version"), "version")?,
            data: integer(fields.get("data"), "data")?,
            parity: integer(fields.get("parity"), "parity")?,
            sets,
        })
    }

    /// The `sets` value written: `[[{ node, zone? }, …], …]`.
    pub(crate) fn sets_value(&self) -> Value {
        Value::Array(
            self.sets
                .iter()
                .map(|set| Value::Array(set.iter().map(MemberEntity::to_value).collect()))
                .collect(),
        )
    }
}

impl MemberEntity {
    fn from_value(value: &Value) -> Result<Self> {
        let fields = fields(value, "layout")?;
        Ok(Self {
            node: text(fields.get("node"), "layout", "node")?,
            zone: optional_text(fields.get("zone"), "layout", "zone")?,
        })
    }

    fn to_value(&self) -> Value {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("node".to_owned(), Value::String(self.node.clone()));
        if let Some(zone) = &self.zone {
            fields.insert("zone".to_owned(), Value::String(zone.clone()));
        }
        Value::Object(fields)
    }
}
