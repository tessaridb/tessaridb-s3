//! `s3_nodes` and `layouts` records exactly as stored.

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

/// One registered node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeEntity {
    /// The node id.
    pub(crate) node: String,
    /// `host:port` of its internal surface.
    pub(crate) endpoint: String,
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
    /// The node ids, one per shard.
    pub(crate) nodes: Vec<String>,
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
        let nodes = match fields.get("nodes") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| text(Some(item), "layout", "nodes"))
                .collect::<Result<Vec<_>>>()?,
            _ => return Err(malformed("layout", "nodes")),
        };
        Ok(Self {
            version: integer(fields.get("version"), "version")?,
            data: integer(fields.get("data"), "data")?,
            parity: integer(fields.get("parity"), "parity")?,
            nodes,
        })
    }
}
