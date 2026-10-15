//! Topology in TessariDB: `s3_nodes:<node id>` per node, `layouts:<version>` per layout and `layout_current:'cluster'`
//! naming the version new objects are written under; every value is bound.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, RefusalClass, Value};

use super::entity::{LayoutEntity, NodeEntity};
use super::repository::{ClusterRepository, Proposed};
use crate::answers::first_record;
use crate::{Error, Result};

/// The TessariDB topology repository.
#[derive(Clone)]
pub struct TessariCluster {
    pool: MetaPool,
}

impl TessariCluster {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

/// Every record of a single-statement read.
fn records(answers: Vec<Answer>) -> Result<Vec<Value>> {
    match answers.into_iter().next() {
        Some(Answer::Records { records, .. }) => {
            Ok(records.into_iter().map(|(_, value)| value).collect())
        }
        Some(Answer::Value {
            value: Value::None, ..
        })
        | Some(Answer::Done) => Ok(Vec::new()),
        Some(Answer::Value { value, .. }) => Ok(vec![value]),
        _ => Err(Error::Malformed {
            record: "cluster",
            reason: "unexpected answer kind",
        }),
    }
}

impl ClusterRepository for TessariCluster {
    async fn register(&self, node: &str, endpoint: &str, zone: Option<&str>) -> Result<()> {
        let parameters = vec![
            ("node".to_owned(), Value::String(node.to_owned())),
            ("endpoint".to_owned(), Value::String(endpoint.to_owned())),
            (
                "zone".to_owned(),
                zone.map_or(Value::None, |zone| Value::String(zone.to_owned())),
            ),
        ];
        self.pool
            .run(
                "UPSERT s3_nodes:$node SET node = $node, endpoint = $endpoint, zone = $zone, \
                 seen = time::now();",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn nodes(&self) -> Result<Vec<NodeEntity>> {
        // 256: the most nodes this server reads a cluster as; a layout uses at most sixteen of them.
        let answers = self
            .pool
            .run(
                "SELECT node, endpoint, zone FROM s3_nodes ORDER BY node LIMIT 256;",
                Vec::new(),
            )
            .await?;
        records(answers)?
            .iter()
            .map(NodeEntity::from_value)
            .collect()
    }

    async fn current(&self) -> Result<Option<i64>> {
        let answers = self
            .pool
            .run(
                "SELECT version FROM ONLY layout_current:'cluster';",
                Vec::new(),
            )
            .await?;
        let Some(record) = first_record(answers.into_iter().next())? else {
            return Ok(None);
        };
        match record {
            Value::Object(fields) => match fields.get("version") {
                Some(Value::Number(Number::Integer(version))) => Ok(Some(*version)),
                _ => Err(Error::Malformed {
                    record: "layout_current",
                    reason: "version",
                }),
            },
            _ => Err(Error::Malformed {
                record: "layout_current",
                reason: "not an object",
            }),
        }
    }

    async fn layout(&self, version: i64) -> Result<Option<LayoutEntity>> {
        let parameters = vec![(
            "version".to_owned(),
            Value::Number(Number::Integer(version)),
        )];
        let answers = self
            .pool
            .run("SELECT * FROM ONLY layouts:$version;", parameters)
            .await?;
        first_record(answers.into_iter().next())?
            .as_ref()
            .map(LayoutEntity::from_value)
            .transpose()
    }

    async fn propose(&self, layout: &LayoutEntity) -> Result<Proposed> {
        let parameters = vec![
            (
                "version".to_owned(),
                Value::Number(Number::Integer(layout.version)),
            ),
            (
                "data".to_owned(),
                Value::Number(Number::Integer(layout.data)),
            ),
            (
                "parity".to_owned(),
                Value::Number(Number::Integer(layout.parity)),
            ),
            ("sets".to_owned(), layout.sets_value()),
        ];
        // One transaction: the first layout never exists without the pointer naming it, nor the pointer without it.
        let script = "BEGIN; CREATE layouts:$version = { version: $version, data: $data, parity: $parity, \
                      sets: $sets, created: time::now() }; CREATE layout_current:'cluster' = { version: $version }; \
                      COMMIT;";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(Proposed::Created),
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(Proposed::Exists),
            Err(error) => Err(error.into()),
        }
    }
}
