//! How a cluster member takes its place at start: its own certificate must speak for it, founding members named in
//! configuration are admitted where no record exists, a join token it was given binds its certificate, and it
//! registers only as an admitted member — so a node nobody approved never becomes part of the topology.

use anyhow::{Context, bail};
use tessari_s3_infrastructure::ClusterSettings;
use tessari_s3_infrastructure::credential::speaks_for_itself;
use tessari_s3_storage::Storage;
use tessari_s3_storage::members::{Fingerprint, JoinToken, Joined};

/// Takes this node's place in the cluster `cluster` describes, before anything is served.
///
/// # Errors
/// Its certificate is another node's, its join token is malformed, it is not an admitted member presenting its
/// certificate, or the metadata store refuses or is unavailable.
pub(crate) async fn take_place(cluster: &ClusterSettings, storage: &Storage) -> anyhow::Result<()> {
    let fingerprint = Fingerprint::new(&speaks_for_itself(&cluster.tls, &cluster.node)?)
        .context("a certificate fingerprint was not in the fingerprint form")?;
    let members = storage.members();
    for founder in &cluster.founding {
        if members
            .found(founder, "founding", "TESSARIDB_S3_FOUNDING_MEMBERS")
            .await?
        {
            tracing::info!(node = founder.as_str(), "founding member admitted");
        }
    }
    if let Some(token) = &cluster.join_token {
        let token = JoinToken::new(token.expose())
            .context("TESSARIDB_S3_JOIN_TOKEN is not a join token: 64 lowercase hex digits")?;
        match members.join(&cluster.node, &token, &fingerprint).await? {
            Joined::Joined => tracing::info!("join token taken: this certificate is the node's"),
            // Used on an earlier start, lapsed, or another node's: the membership as it stands decides below.
            Joined::Refused => tracing::warn!("the join token was not taken"),
        }
    }
    let registered = storage
        .cluster()
        .register(
            &cluster.node,
            &fingerprint,
            &cluster.internal_advertise,
            cluster.zone.as_ref(),
        )
        .await?;
    if !registered {
        bail!(
            "node {} is not an admitted member presenting this certificate: approve it in the console, name it in \
             TESSARIDB_S3_FOUNDING_MEMBERS on a new cluster, or start it with a join token issued for it",
            cluster.node.as_str()
        );
    }
    Ok(())
}
