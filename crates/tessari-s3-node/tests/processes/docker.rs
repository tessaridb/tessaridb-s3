//! Docker and the credentials the metadata cluster runs with: one-shot containers over a store's volume, and a
//! certificate authority minted for one cluster that issues each node its peer credential.

use std::path::Path;
use std::process::Command as BlockingCommand;

use tokio::process::Command;

/// The engine release the metadata runs on, as the README pins it.
pub const IMAGE: &str = "tessaridb/tessaridb:0.33.2-beta";

/// Runs `docker` with `args` and answers its standard output; any failure fails the test with docker's own words.
pub async fn docker(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .await
        .expect("docker runs");
    assert!(
        output.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The identifier a fresh store on `volume` gives itself, read off `INFO FOR NODE` as the node prints it.
pub async fn identity(volume: &str) -> String {
    let mount = format!("{volume}:/var/lib/tessaridb");
    let info = docker(&[
        "run",
        "--rm",
        "-v",
        &mount,
        IMAGE,
        "/var/lib/tessaridb/store",
        "-e",
        "INFO FOR NODE;",
    ])
    .await;
    let id = info
        .split(" id: '")
        .nth(1)
        .and_then(|rest| rest.get(..32))
        .unwrap_or_else(|| panic!("no id in {info}"));
    id.to_owned()
}

/// A certificate authority minted for one cluster, the peer credential it issues each node, and where they are.
pub fn mint(ids: &[String], into: &Path) {
    let mut authority = rcgen::CertificateParams::new(Vec::new()).expect("authority parameters");
    authority.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let authority_key = rcgen::KeyPair::generate().expect("authority key");
    let authority = authority
        .self_signed(&authority_key)
        .expect("authority certificate");
    std::fs::write(into.join("ca.pem"), authority.pem()).expect("authority written");
    for (index, id) in ids.iter().enumerate() {
        // The name the engine asks a peer's credential for: `<node id>.peer.tessari`.
        let leaf = rcgen::CertificateParams::new(vec![format!("{id}.peer.tessari")])
            .expect("leaf parameters");
        let key = rcgen::KeyPair::generate().expect("leaf key");
        let leaf = leaf
            .signed_by(&key, &authority, &authority_key)
            .expect("leaf certificate");
        std::fs::write(into.join(format!("n{index}.pem")), leaf.pem()).expect("leaf written");
        // Readable here so the container can copy it; the copy it uses is made owner-only inside it.
        std::fs::write(into.join(format!("n{index}.key")), key.serialize_pem())
            .expect("key written");
    }
}

/// Whether Docker answers and the pinned image is present, for a clear failure before anything is created.
pub fn available() -> Result<(), String> {
    let output = BlockingCommand::new("docker")
        .args(["image", "inspect", IMAGE])
        .output()
        .map_err(|error| format!("docker does not run: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "the image {IMAGE} is not present: docker pull {IMAGE}"
        ))
    }
}
