//! Topology in TessariDB: `s3_nodes:<node id>` per node and `layouts:<version>` per layout; every value is bound.

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
    async fn register(&self, node: &str, endpoint: &str) -> Result<()> {
        let parameters = vec![
            ("node".to_owned(), Value::String(node.to_owned())),
            ("endpoint".to_owned(), Value::String(endpoint.to_owned())),
        ];
        self.pool
            .run(
                "UPSERT s3_nodes:$node SET node = $node, endpoint = $endpoint, seen = time::now();",
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
                "SELECT node, endpoint FROM s3_nodes ORDER BY node LIMIT 256;",
                Vec::new(),
            )
            .await?;
        records(answers)?
            .iter()
            .map(NodeEntity::from_value)
            .collect()
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
            (
                "nodes".to_owned(),
                Value::Array(layout.nodes.iter().cloned().map(Value::String).collect()),
            ),
        ];
        let script = "CREATE layouts:$version = { version: $version, data: $data, parity: $parity, nodes: $nodes, \
                      created: time::now() };";
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(Proposed::Created),
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(Proposed::Exists),
            Err(error) => Err(error.into()),
        }
    }
}
