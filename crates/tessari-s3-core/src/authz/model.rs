//! The evaluator's inputs: who is asking, what they want to do, and to which bucket.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::policy::{BucketPolicy, PolicyError, PublicAccessBlock};

/// A space's name: the tenant buckets and users belong to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpaceName(String);

/// Whether `name` is 1-63 lowercase letters, digits and inner hyphens, as node names are.
fn is_node_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

impl SpaceName {
    /// The space called `name`; names are 1-63 lowercase letters, digits and inner hyphens, as node names are.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        is_node_name(name).then(|| Self(name.to_owned()))
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

/// A user's name, unique across the store; the same rule as a space's name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserName(String);

impl UserName {
    /// The user called `name`; names are 1-63 lowercase letters, digits and inner hyphens.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        is_node_name(name).then(|| Self(name.to_owned()))
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A group's name, unique within its space; the same rule as a user's name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupName(String);

impl GroupName {
    /// The group called `name`; names are 1-63 lowercase letters, digits and inner hyphens.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        is_node_name(name).then(|| Self(name.to_owned()))
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

impl Access {
    /// What either grant allows: a user's own grant on a bucket joined with those of their groups.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self {
            read: self.read || other.read,
            write: self.write || other.write,
        }
    }
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

/// A credential a user issued to upload ONE key of one bucket: it carries its issuer, and its authority is the
/// issuer's, narrowed to that key — never more than either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPrincipal {
    /// The user who issued it, resolved afresh like any user; disabling them or taking their grant ends the key.
    pub parent: UserPrincipal,
    /// The one bucket it may write, by name.
    pub bucket: String,
    /// The one key it may write, compared exactly.
    pub key: String,
}

/// Temporary credentials a user was issued through the identity provider: the user's authority and nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPrincipal {
    /// The user, as the principal every decision of the session is taken for.
    as_user: Box<Principal>,
}

impl SessionPrincipal {
    /// A session of `user`, resolved afresh like any user: disabling them or taking a grant binds the session too.
    #[must_use]
    pub fn new(user: UserPrincipal) -> Self {
        Self {
            as_user: Box::new(Principal::User(user)),
        }
    }

    /// The user the session acts as.
    #[must_use]
    pub fn as_user(&self) -> &Principal {
        &self.as_user
    }

    /// The user, as a creator or a space is named for.
    #[must_use]
    pub fn user(&self) -> Option<&UserPrincipal> {
        match self.as_user.as_ref() {
            Principal::User(user) => Some(user),
            _ => None,
        }
    }
}

/// Who is asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// The configured root credential: every action, as before spaces existed.
    Root,
    /// A user of a space.
    User(UserPrincipal),
    /// A one-key upload credential.
    Upload(UploadPrincipal),
    /// Temporary credentials from the identity provider.
    Session(SessionPrincipal),
    /// A request that carried no credentials: reaches only what a bucket policy grants everyone.
    Anonymous,
}

impl Principal {
    /// Which buckets a listing may return to this principal; storage applies it in the query, never afterwards.
    #[must_use]
    pub fn visible(&self) -> Visible {
        match self {
            Self::User(user) if user.operator => Visible::All,
            Self::User(user) if user.role == Role::SpaceAdmin => Visible::Space(user.space.clone()),
            Self::User(user) => Visible::Reachable {
                space: user.space.clone(),
                user: user.name.clone(),
                granted: user
                    .grants
                    .iter()
                    .filter(|(_, access)| access.read || access.write)
                    .map(|(bucket, _)| bucket.clone())
                    .collect(),
            },
            Self::Root => Visible::All,
            Self::Session(session) => session.as_user().visible(),
            // A one-key credential lists nothing: no user is called "", and it is granted no bucket.
            Self::Upload(upload) => Visible::Reachable {
                space: upload.parent.space.clone(),
                user: String::new(),
                granted: Vec::new(),
            },
            // Nobody lists every bucket anonymously; listing is refused before it gets here.
            Self::Anonymous => Visible::Reachable {
                space: SpaceName::default_space(),
                user: String::new(),
                granted: Vec::new(),
            },
        }
    }
}

/// The part of the store a principal can list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visible {
    /// Every space: root and operators.
    All,
    /// One space's buckets: its administrator.
    Space(SpaceName),
    /// The buckets of one space a member can reach: those it created and those a grant lets it read or write.
    Reachable {
        /// The member's space.
        space: SpaceName,
        /// The member, matched against each bucket's creator.
        user: String,
        /// The buckets its grants reach, by name.
        granted: Vec<String>,
    },
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
    /// Its policy, parsed when the bucket was read; `Err` when the stored text no longer parses.
    pub policy: Option<Arc<Result<BucketPolicy, PolicyError>>>,
    /// Its Block Public Access switches, as in force (every one on while none is stored).
    pub public_access: PublicAccessBlock,
}

/// A user as the evaluator needs it when somebody manages them: who they are, where, and with what authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserResource {
    /// Their name.
    pub name: String,
    /// The space they belong to.
    pub space: SpaceName,
    /// Their role in it.
    pub role: Role,
    /// Whether they operate the store.
    pub operator: bool,
    /// Whether they see the cluster.
    pub cluster_viewer: bool,
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
    /// Change a bucket's configuration — who may delete it may configure it; a grant confers neither.
    ConfigureBucket(BucketResource),
    /// Describe a bucket or list what it holds.
    ReadBucket(BucketResource),
    /// Read an object of a bucket.
    ReadObject(BucketResource),
    /// Write, overwrite, upload parts to or delete an object of a bucket.
    WriteObject(BucketResource),
    /// Upload one named key of a bucket: a single PUT or the steps of a multipart upload of it.
    Upload {
        /// The bucket written.
        bucket: BucketResource,
        /// The key written.
        key: String,
    },
    /// See the cluster's members, drives, layout and healing.
    ViewCluster,
    /// Administer a space's users and grants.
    ManageSpace(SpaceName),
    /// Create a user, issue it a key, disable it or change its grants — judged on the user as it is or would be.
    ManageUser(UserResource),
    /// Create and remove spaces, set quotas, read every space, change the cluster's layout.
    Operate,
}

impl Action {
    /// The bucket the action is on, if it is on one.
    #[must_use]
    pub const fn bucket(&self) -> Option<&BucketResource> {
        match self {
            Self::DeleteBucket(bucket)
            | Self::ConfigureBucket(bucket)
            | Self::ReadBucket(bucket)
            | Self::ReadObject(bucket)
            | Self::WriteObject(bucket)
            | Self::Upload { bucket, .. } => Some(bucket),
            _ => None,
        }
    }
}
