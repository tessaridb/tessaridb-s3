//! Space rules: creating, finding and listing spaces.

use tessari_s3_core::authz::SpaceName;

use super::model::Space;
use super::repository::{Inserted, SpaceRepository};
use super::tessaridb::TessariSpaces;
use crate::Result;

/// What creating a space ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceCreated {
    /// The space now exists.
    Created(Space),
    /// A space of that name already existed.
    Exists,
}

/// Space operations.
#[derive(Clone)]
pub struct SpaceService {
    repository: TessariSpaces,
}

impl SpaceService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariSpaces) -> Self {
        Self { repository }
    }

    /// Creates the space `name`.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn create(&self, name: &SpaceName) -> Result<SpaceCreated> {
        match self.repository.insert(name).await? {
            Inserted::Created(entity) => Ok(SpaceCreated::Created(Space::try_from(entity)?)),
            Inserted::Exists => Ok(SpaceCreated::Exists),
        }
    }

    /// The space called `name`, if it exists.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn get(&self, name: &SpaceName) -> Result<Option<Space>> {
        self.repository
            .get(name)
            .await?
            .map(Space::try_from)
            .transpose()
    }

    /// Every space, in byte order of their names.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn list(&self) -> Result<Vec<Space>> {
        let mut spaces: Vec<Space> = self
            .repository
            .list()
            .await?
            .into_iter()
            .map(Space::try_from)
            .collect::<Result<_>>()?;
        spaces.sort_by(|a, b| a.name.as_str().as_bytes().cmp(b.name.as_str().as_bytes()));
        Ok(spaces)
    }
}
