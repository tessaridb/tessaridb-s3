//! The evaluator's inputs: who is asking, what they want to do, and to which bucket.

use std::collections::BTreeMap;

/// A space's name: the tenant buckets and users belong to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpaceName(String);

impl SpaceName {
    /// The space called `name`; names are 1-63 lowercase letters, digits and inner hyphens, as node names are.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        let valid = (1..=63).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !name.starts_with('-')
            && !name.ends_with('-');
        valid.then(|| Self(name.to_owned()))
    }

    /// The space buckets from before spaces existed belong to, and the one the root credential creates in.
    #[must_use]
    pub fn default_space() -> Self {
        Self(tessari_s3_constants::DEFAULT_SPACE.to_owned())
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A user's role inside their space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Administers the space: its users, its buckets and their grants.
    SpaceAdmin,
    /// Reaches the buckets it created and those it was granted.
    Member,
}

/// What a grant on one bucket allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Access {
    /// Listing and reading objects.
    pub read: bool,
    /// Writing and deleting objects.
    pub write: bool,
}

/// A signed-in user as the evaluator sees them: resolved from the store, never from the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPrincipal {
    /// The user's name.
    pub name: String,
    /// The one space they belong to.
    pub space: SpaceName,
    /// Their role in it.
    pub role: Role,
    /// Whether a member may create buckets in the space.
    pub create_buckets: bool,
    /// Store-wide: every space, users, quotas.
    pub operator: bool,
    /// Store-wide: the cluster's members, drives and healing.
    pub cluster_viewer: bool,
    /// Per-bucket grants, by bucket name.
    pub grants: BTreeMap<String, Access>,
}

/// Who is asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// The configured root credential: every action, as before spaces existed.
    Root,
    /// A user of a space.
    User(UserPrincipal),
}

impl Principal {
    /// Which buckets a listing may return to this principal; storage applies it in the query, never afterwards.
    #[must_use]
    pub fn visible(&self) -> Visible {
        match self {
            Self::User(user) if !user.operator => Visible::Space(user.space.clone()),
            Self::Root | Self::User(_) => Visible::All,
        }
    }
}

/// The part of the store a principal can list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visible {
    /// Every space: root and operators.
    All,
    /// One space's buckets.
    Space(SpaceName),
}

/// A bucket as the evaluator needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketResource {
    /// Its name.
    pub name: String,
    /// The space that owns it.
    pub space: SpaceName,
    /// The user who created it, when a user did.
    pub creator: Option<String>,
}

/// What is being asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// List the buckets the caller can see (the listing itself is filtered by space in the query).
    ListBuckets,
    /// Create a bucket in a space.
    CreateBucket(SpaceName),
    /// Delete a bucket.
    DeleteBucket(BucketResource),
    /// Describe a bucket or list what it holds.
    ReadBucket(BucketResource),
    /// Read an object of a bucket.
    ReadObject(BucketResource),
    /// Write, overwrite, upload parts to or delete an object of a bucket.
    WriteObject(BucketResource),
    /// See the cluster's members, drives, layout and healing.
    ViewCluster,
    /// Administer a space's users and grants.
    ManageSpace(SpaceName),
    /// Create and remove spaces, set quotas, read every space.
    Operate,
}
