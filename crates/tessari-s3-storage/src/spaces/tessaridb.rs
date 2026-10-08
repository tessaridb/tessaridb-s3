//! Spaces in TessariDB: one `spaces` record per space, id = the space name; every value is bound.

use tessari_s3_core::authz::SpaceName;
use tessari_s3_infrastructure::tessaridb::{MetaPool, RefusalClass, Value};

use super::entity::SpaceEntity;
use super::repository::{Inserted, SpaceRepository};
use crate::answers::{all_records, first_record};
use crate::{Error, Result};

/// The TessariDB space repository.
#[derive(Clone)]
pub struct TessariSpaces {
    pool: MetaPool,
}

impl TessariSpaces {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn name_param(name: &SpaceName) -> Vec<(String, Value)> {
    vec![("name".to_owned(), Value::String(name.as_str().to_owned()))]
}

impl SpaceRepository for TessariSpaces {
    async fn insert(&self, name: &SpaceName) -> Result<Inserted> {
        let script = "CREATE spaces:$name = { name: $name, created: time::now() } RETURN AFTER;";
        match self.pool.run(script, name_param(name)).await {
            Ok(answers) => match first_record(answers.into_iter().next())? {
                Some(value) => Ok(Inserted::Created(SpaceEntity::from_value(&value)?)),
                None => Err(Error::Malformed {
                    record: "space",
                    reason: "CREATE answered no record",
                }),
            },
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(Inserted::Exists),
            Err(error) => Err(error.into()),
        }
    }

    async fn get(&self, name: &SpaceName) -> Result<Option<SpaceEntity>> {
        let answers = self
            .pool
            .run("SELECT * FROM ONLY spaces:$name;", name_param(name))
            .await?;
        first_record(answers.into_iter().next())?
            .map(|value| SpaceEntity::from_value(&value))
            .transpose()
    }

    async fn list(&self) -> Result<Vec<SpaceEntity>> {
        let answers = self.pool.run("SELECT * FROM spaces;", Vec::new()).await?;
        all_records(answers.into_iter().next(), "space")?
            .iter()
            .map(SpaceEntity::from_value)
            .collect()
    }
}
