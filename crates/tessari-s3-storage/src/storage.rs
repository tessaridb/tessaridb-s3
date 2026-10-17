//! [`Storage`]: the services the API reaches persistence through, and nothing else.

use std::path::PathBuf;
use std::sync::Arc;

use tessari_s3_constants::{DATA_BLOCK_SIZE, DEFAULT_RECLAIM_GRACE_SECS};
use tessari_s3_core::authz::SpaceName;
use tessari_s3_infrastructure::ClusterSettings;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_types::IamKey;

use crate::Result;
use crate::actions::{ActionService, TessariActions};
use crate::audit::{AuditService, TessariAudit};
use crate::buckets::{BucketService, TessariBuckets};
use crate::cluster::{ClusterService, TessariCluster};
use crate::data::DataFiles;
use crate::erasure::ErasureWrites;
use crate::groups::{GroupService, TessariGroups};
use crate::identities::{IdentityService, TessariIdentities};
use crate::keys::{KeyService, TessariKeys};
use crate::lifecycle::{LifecycleService, TessariLifecycle};
use crate::media::{MediaService, TessariMedia};
use crate::members::{MembershipService, TessariMembers};
use crate::multipart::{MultipartService, TessariMultipart};
use crate::notifications::{NotificationService, TessariNotifications};
use crate::objects::{ObjectService, TessariObjects};
use crate::peers::RemoteShards;
use crate::regions::{RegionService, TessariRegions};
use crate::replication::{ReplicationService, TessariReplication};
use crate::shards::ShardService;
use crate::spaces::{SpaceService, TessariSpaces};
use crate::usage::{TessariUsage, UsageService};
use crate::users::{TessariUsers, UserService};

/// Every storage service, built over one metadata pool: the global database for buckets and identities, this node's
/// region's database for everything else.
#[derive(Clone)]
pub struct Storage {
    buckets: BucketService,
    objects: ObjectService,
    multipart: MultipartService,
    cluster: ClusterService,
    members: MembershipService,
    shards: ShardService,
    actions: ActionService,
    usage: UsageService,
    spaces: SpaceService,
    users: UserService,
    groups: GroupService,
    identities: IdentityService,
    /// Shared by every clone: an unseal on this node is seen by all of them.
    keys: Arc<KeyService>,
    pool: MetaPool,
    regional: MetaPool,
    reclaim_grace_secs: u64,
}

impl Storage {
    /// The services over `pool` for a node of `region`, with every object's bytes under `data_dir`. Nothing is
    /// dialled or created until the first call.
    ///
    /// # Errors
    /// [`crate::Error::Meta`] when `region` does not name a database.
    pub fn new(pool: MetaPool, region: &str, data_dir: PathBuf) -> Result<Self> {
        let regional = pool.regional(region)?;
        let data = DataFiles::new(data_dir, DATA_BLOCK_SIZE);
        let shards = ShardService::new(data.clone());
        let keys = Arc::new(KeyService::new(TessariKeys::new(pool.clone())));
        let objects = ObjectService::new(
            TessariObjects::new(regional.clone()),
            data,
            Arc::clone(&keys),
        );
        let spaces = SpaceService::new(TessariSpaces::new(pool.clone()));
        let groups = GroupService::new(TessariGroups::new(pool.clone()));
        Ok(Self {
            buckets: BucketService::new(TessariBuckets::new(pool.clone(), regional.clone())),
            multipart: MultipartService::new(
                TessariMultipart::new(regional.clone()),
                objects.clone(),
            ),
            cluster: ClusterService::new(TessariCluster::new(regional.clone())),
            members: MembershipService::new(TessariMembers::new(regional.clone())),
            actions: ActionService::new(TessariActions::new(pool.clone())),
            usage: UsageService::new(TessariUsage::new(regional.clone())),
            users: UserService::new(
                TessariUsers::new(pool.clone()),
                spaces.clone(),
                groups.clone(),
            ),
            groups,
            identities: IdentityService::new(TessariIdentities::new(pool.clone())),
            keys,
            spaces,
            objects,
            shards,
            pool,
            regional,
            reclaim_grace_secs: DEFAULT_RECLAIM_GRACE_SECS,
        })
    }

    /// The services of a cluster member: as [`Storage::new`], with every data id written as erasure-coded shards
    /// over the cluster described by `cluster`.
    ///
    /// # Errors
    /// [`crate::Error::Peer`] when the peer client cannot be built.
    pub fn clustered(
        pool: MetaPool,
        region: &str,
        data_dir: PathBuf,
        cluster: &ClusterSettings,
    ) -> Result<Self> {
        let mut storage = Self::new(pool, region, data_dir)?;
        let regional = storage.regional.clone();
        let peers = RemoteShards::new(cluster.node.clone(), &cluster.tls)?;
        storage.cluster = storage.cluster.clone().with_peers(peers.clone());
        let writes = ErasureWrites::new(
            cluster,
            regional.clone(),
            storage.cluster.clone(),
            storage.shards.clone(),
            peers,
        );
        storage.objects = storage.objects.clone().clustered(Arc::new(writes));
        storage.usage = storage.usage.clone().clustered(cluster.code)?;
        storage.multipart =
            MultipartService::new(TessariMultipart::new(regional), storage.objects.clone());
        Ok(storage)
    }

    /// The same services with user access-key secrets sealed and opened under `root`; `None` (no
    /// `TESSARIDB_S3_IAM_KEY`) leaves the root credential as the only one that works.
    #[must_use]
    pub fn with_iam_key(self, root: Option<IamKey>) -> Self {
        Self {
            users: self.users.with_iam_key(root),
            ..self
        }
    }

    /// The same services with a queued data file held back `secs` seconds before it may be reclaimed (unset:
    /// [`DEFAULT_RECLAIM_GRACE_SECS`]); [`Storage::prepare`] records it in the store, where every queuing reads it.
    #[must_use]
    pub fn with_reclaim_grace(self, secs: u64) -> Self {
        Self {
            reclaim_grace_secs: secs,
            ..self
        }
    }

    /// Creates the schema when it is missing, moves an earlier build's work lists onto the queues, creates the
    /// default space, and clears what writes cut short by a crash left on the drive; run once at start-up, before the
    /// node serves.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or the drive's.
    pub async fn prepare(&self) -> Result<()> {
        crate::schema::apply(&self.pool, &self.regional, self.reclaim_grace_secs).await?;
        let moved = crate::schema_queues::convert(&self.regional).await?;
        if moved > 0 {
            tracing::info!(moved, "work-list entries moved onto the queues");
        }
        self.spaces.create(&SpaceName::default_space()).await?;
        let cleared = self.objects.recover().await?;
        if cleared > 0 {
            tracing::info!(cleared, "temporary files of interrupted writes removed");
        }
        Ok(())
    }

    /// Object operations.
    #[must_use]
    pub const fn objects(&self) -> &ObjectService {
        &self.objects
    }

    /// Multipart upload operations.
    #[must_use]
    pub const fn multipart(&self) -> &MultipartService {
        &self.multipart
    }

    /// Cluster topology: registered nodes, whether they answer, and the layout.
    #[must_use]
    pub const fn cluster(&self) -> &ClusterService {
        &self.cluster
    }

    /// The encryption keys this node holds, sealed until an operator unseals them.
    #[must_use]
    pub fn keys(&self) -> &KeyService {
        &self.keys
    }

    /// Which nodes an operator admitted to the cluster, and which were removed.
    #[must_use]
    pub const fn members(&self) -> &MembershipService {
        &self.members
    }

    /// This node's shards of erasure-coded objects.
    #[must_use]
    pub const fn shards(&self) -> &ShardService {
        &self.shards
    }

    /// The lifecycle engine, over this node's bucket, object and multipart services.
    #[must_use]
    pub fn lifecycle(&self) -> LifecycleService {
        LifecycleService::new(
            TessariLifecycle::new(self.regional.clone()),
            self.buckets.clone(),
            self.objects.clone(),
            self.multipart.clone(),
        )
    }

    /// The region's access log: one record per S3 request.
    #[must_use]
    pub fn audit(&self) -> AuditService {
        AuditService::new(TessariAudit::new(self.regional.clone()))
    }

    /// Delivery of bucket event notifications to the operator's declared destinations.
    #[must_use]
    pub fn notifications(&self) -> NotificationService {
        NotificationService::new(
            TessariNotifications::new(self.regional.clone()),
            self.buckets.clone(),
        )
    }

    /// Which regions are failed over, and failing a region over or restoring it.
    #[must_use]
    pub fn regions(&self) -> RegionService {
        RegionService::new(TessariRegions::new(self.pool.clone()))
    }

    /// Replication of this region's versions to their destination buckets.
    #[must_use]
    pub fn replication(&self) -> ReplicationService {
        ReplicationService::new(TessariReplication::new(
            self.pool.clone(),
            self.regional.clone(),
        ))
    }

    /// Media jobs: objects created where a bucket's media rules may take them.
    #[must_use]
    pub fn media(&self) -> MediaService {
        MediaService::new(
            TessariMedia::new(self.regional.clone()),
            self.buckets.clone(),
        )
    }

    /// Bucket operations.
    #[must_use]
    pub const fn buckets(&self) -> &BucketService {
        &self.buckets
    }

    /// The record of what operators did through the console.
    #[must_use]
    pub const fn actions(&self) -> &ActionService {
        &self.actions
    }

    /// How many objects and bytes each bucket holds, as last measured.
    #[must_use]
    pub const fn usage(&self) -> &UsageService {
        &self.usage
    }

    /// The spaces buckets and users belong to.
    #[must_use]
    pub const fn spaces(&self) -> &SpaceService {
        &self.spaces
    }

    /// Users, their access keys and their grants.
    #[must_use]
    pub const fn users(&self) -> &UserService {
        &self.users
    }

    /// The groups of each space and what they grant their members.
    #[must_use]
    pub const fn groups(&self) -> &GroupService {
        &self.groups
    }

    /// Console sign-ins in flight to the identity provider, and which user each provider identity is.
    #[must_use]
    pub const fn identities(&self) -> &IdentityService {
        &self.identities
    }
}
