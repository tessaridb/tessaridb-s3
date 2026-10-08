//! A three-node TessariDB cluster in Docker, for the tests that kill its leader.
//!
//! Each node is a container of the pinned image on its own volume and a fixed address in a network of its own:
//! peers greet each other over mutual TLS with credentials a test-minted authority issued for each node's identifier,
//! and every membership row names where the HOST reaches that node, so a write sent to a follower is redirected
//! somewhere the S3 processes can follow. All three stores carry the same founding transaction before any starts;
//! the owner is declared once over the wire, after a leader is elected, so the three logs never disagree. Everything
//! is named `tessari-s3-meta-<run>-…` and removed when the cluster is dropped.

use std::path::PathBuf;
use std::process::Command as BlockingCommand;
use std::time::{Duration, Instant};

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};
use tokio::process::Command;

use crate::docker::{IMAGE, docker, identity, mint};
use crate::s3::{Meta, free_port, scratch};

const USER: &str = "owner";
const PASSWORD: &str = "meta-cluster-pass-0123456789";
/// How long the cluster may take to elect its first leader and take the owner's declaration.
const ELECTED_WITHIN: Duration = Duration::from_secs(60);
const ROLES: &str = "serving, writable, coordinating";

/// One running node: its container, its volume, its address in the network and the one the host reaches it at.
struct MetaNode {
    container: String,
    volume: String,
    ip: String,
    wire: String,
    id: String,
}

pub struct MetaCluster {
    network: String,
    nodes: Vec<MetaNode>,
    credentials: PathBuf,
}

impl Drop for MetaCluster {
    fn drop(&mut self) {
        // Blocking, in a destructor that runs once per test: only what this cluster created, by name.
        for node in &self.nodes {
            let _ = BlockingCommand::new("docker")
                .args(["rm", "-f", &node.container])
                .output();
            let _ = BlockingCommand::new("docker")
                .args(["volume", "rm", "-f", &node.volume])
                .output();
        }
        let _ = BlockingCommand::new("docker")
            .args(["network", "rm", &self.network])
            .output();
        std::fs::remove_dir_all(&self.credentials).ok();
    }
}

/// A single-address pool to node `wire`, signed in as the owner.
fn pool(wire: &str) -> MetaPool {
    let base = Meta {
        addresses: wire.to_owned(),
        user: USER.to_owned(),
        password: PASSWORD.to_owned(),
        replication: None,
    };
    MetaPool::new(crate::s3::settings(&base, "probe")).expect("pool")
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Object(fields) => fields.get(name),
        _ => None,
    }
}

impl MetaCluster {
    pub async fn start() -> Self {
        let run = uuid::Uuid::new_v4().simple().to_string();
        let prefix = format!("tessari-s3-meta-{}", &run[..12]);
        let network = format!("{prefix}-net");
        docker(&["network", "create", &network]).await;
        let credentials = scratch("meta-credentials");
        std::fs::create_dir_all(&credentials).expect("credentials directory");
        let mut cluster = Self {
            network,
            nodes: Vec::new(),
            credentials,
        };
        let subnet = docker(&[
            "network",
            "inspect",
            &cluster.network,
            "-f",
            "{{(index .IPAM.Config 0).Subnet}}",
        ])
        .await;
        let base: Vec<&str> = subnet.split('.').take(3).collect();
        for index in 0..3_u8 {
            let volume = format!("{prefix}-n{index}");
            docker(&["volume", "create", &volume]).await;
            let id = identity(&volume).await;
            cluster.nodes.push(MetaNode {
                container: format!("{prefix}-n{index}"),
                volume,
                // A peer is reached at an address, never a name: the leadership round refuses a host name.
                ip: format!("{}.{}", base.join("."), index.saturating_add(10)),
                wire: format!("127.0.0.1:{}", free_port()),
                id,
            });
        }
        let ids: Vec<String> = cluster.nodes.iter().map(|node| node.id.clone()).collect();
        mint(&ids, &cluster.credentials);
        cluster.found().await;
        for index in 0..cluster.nodes.len() {
            cluster.serve(index).await;
        }
        cluster.declare_owner().await;
        cluster.signed_in_everywhere().await;
        cluster
    }

    /// Waits until every node has collected the owner: one that has not refuses the sign-in.
    async fn signed_in_everywhere(&self) {
        let started = Instant::now();
        for node in &self.nodes {
            while let Err(error) = pool(&node.wire).run_unscoped("RETURN 1;").await {
                assert!(
                    started.elapsed() < ELECTED_WITHIN,
                    "{} never took the owner's sign-in: {error}",
                    node.container
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    /// The same membership on every store before any of them starts: a node that only says its roles and joins
    /// later comes up as a writable store of its own and never collects.
    async fn found(&self) {
        let rows: String = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                format!(
                    "DEFINE REPLICA n{index} AT '{}:9000' CLIENTS AT '{}' NODE '{}' ROLES {ROLES} \
                     REPLICATES STORE; ",
                    node.ip, node.wire, node.id
                )
            })
            .collect();
        let script = format!("BEGIN; {rows}DEFINE NODE ROLES {ROLES}; COMMIT;");
        for node in &self.nodes {
            let mount = format!("{}:/var/lib/tessaridb", node.volume);
            docker(&[
                "run",
                "--rm",
                "-v",
                &mount,
                IMAGE,
                "/var/lib/tessaridb/store",
                "-e",
                &script,
            ])
            .await;
        }
    }

    /// Starts node `index`, seeded with the next one.
    async fn serve(&self, index: usize) {
        let node = &self.nodes[index];
        let seed = &self.nodes[index
            .saturating_add(1)
            .checked_rem(self.nodes.len())
            .unwrap_or_default()];
        let mount = format!("{}:/var/lib/tessaridb", node.volume);
        let credentials = format!("{}:/credentials:ro", self.credentials.display());
        let publish = format!("{}:9080", node.wire);
        // The key is copied out of the bind mount and made owner-only, which the node demands of a key file.
        let start = format!(
            "mkdir -p /tmp/k && cp /credentials/n{index}.pem /credentials/n{index}.key /credentials/ca.pem /tmp/k/ \
             && chmod 600 /tmp/k/n{index}.key && exec tessaridb /var/lib/tessaridb/store --serve 0.0.0.0:9080 \
             --cluster-credential /tmp/k/n{index}.pem --cluster-key /tmp/k/n{index}.key \
             --cluster-authority /tmp/k/ca.pem --cluster-address 0.0.0.0:9000 --seed {}@{}:9000",
            seed.id, seed.ip
        );
        docker(&[
            "run",
            "-d",
            "--name",
            &node.container,
            "--network",
            &self.network,
            "--ip",
            &node.ip,
            "-p",
            &publish,
            "-v",
            &mount,
            "-v",
            &credentials,
            "--entrypoint",
            "/bin/sh",
            IMAGE,
            "-c",
            &start,
        ])
        .await;
    }

    /// Closes the store: declared once, through any node, once a leader takes writes.
    async fn declare_owner(&self) {
        let started = Instant::now();
        let mut last = String::new();
        loop {
            for node in &self.nodes {
                let answered = async {
                    let mut client = tessaridb_client::Client::connect(node.wire.as_str()).await?;
                    // The grammar takes the password as text and refuses a parameter there; it is this test's
                    // own constant, never a value from outside.
                    client
                        .run(
                            &format!("DEFINE USER {USER} ROLE owner PASSWORD '{PASSWORD}';"),
                            None,
                        )
                        .await
                }
                .await;
                match answered {
                    Ok(_) => return,
                    Err(error) => last = error.to_string(),
                }
            }
            assert!(
                started.elapsed() < ELECTED_WITHIN,
                "no node took the owner's declaration within {ELECTED_WITHIN:?}: {last}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// The settings the S3 processes reach this cluster with: every node, three copies of their namespace.
    pub fn meta(&self) -> Meta {
        Meta {
            addresses: self
                .nodes
                .iter()
                .map(|node| node.wire.clone())
                .collect::<Vec<_>>()
                .join(","),
            user: USER.to_owned(),
            password: PASSWORD.to_owned(),
            replication: Some("3".to_owned()),
        }
    }

    /// Which nodes hold a live lease, by index; a node that is down or holds none is not among them.
    pub async fn leaders(&self) -> Vec<usize> {
        let mut leading = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            let Ok(answers) = pool(&node.wire).run_unscoped("INFO FOR NODE;").await else {
                continue;
            };
            let lease = answers.iter().find_map(|answer| match answer {
                Answer::Value { value, .. } => {
                    field(value, "cluster").and_then(|cluster| field(cluster, "lease"))
                }
                _ => None,
            });
            if matches!(lease, Some(Value::Duration { .. })) {
                leading.push(index);
            }
        }
        leading
    }

    /// The one node leading, waited for.
    pub async fn leader(&self) -> usize {
        let started = Instant::now();
        loop {
            if let [one] = self.leaders().await.as_slice() {
                return *one;
            }
            assert!(
                started.elapsed() < ELECTED_WITHIN,
                "no single leader within {ELECTED_WITHIN:?}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Kills node `index` outright, as a power cut would.
    pub async fn kill(&self, index: usize) {
        docker(&["kill", &self.nodes[index].container]).await;
    }

    /// The last lines node `index` wrote, for a failure message.
    pub async fn said(&self, index: usize) -> String {
        let output = Command::new("docker")
            .args(["logs", "--tail", "30", &self.nodes[index].container])
            .output()
            .await
            .expect("docker runs");
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }
}
