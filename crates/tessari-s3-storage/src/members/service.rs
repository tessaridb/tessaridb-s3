//! Membership rules: an operator admits a node by its id, by one certificate's fingerprint, or by a one-time join
//! token the node offers when it starts; a removed node is a tombstone that is never admitted again; and a node the
//! current layout still places shards on is not removed, because its shards would become unreachable.

use sha2::{Digest, Sha256};
use tessari_s3_core::erasure::Layout;
use tessari_s3_types::{NodeId, Timestamp};
use zeroize::Zeroizing;

use super::entity::MemberEntity;
use super::model::{
    Admission, Approval, Fingerprint, Issued, JoinToken, Joined, MemberState, Membership, Removal,
    TokenLifetime,
};
use super::repository::{Grant, MemberRepository};
use super::tessaridb::TessariMembers;
use crate::{Error, Result};

const HEX: &[u8; 16] = b"0123456789abcdef";

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// The SHA-256 a token is stored as. A digest compared in the statement is no timing oracle: nobody steers either
/// side of a hash of 256 random bits.
fn digest(token: &JoinToken) -> String {
    hex(&Sha256::digest(token.expose().as_bytes()))
}

/// A new token: 32 bytes of the operating system's random source as 64 hex digits.
fn draw() -> Result<JoinToken> {
    let mut bytes = Zeroizing::new([0_u8; 32]);
    getrandom::fill(bytes.as_mut_slice()).map_err(|_| Error::Randomness)?;
    let text = Zeroizing::new(hex(bytes.as_slice()));
    JoinToken::new(&text).ok_or(Error::Randomness)
}

fn membership(entity: MemberEntity) -> Result<Membership> {
    let malformed = |reason| Error::Malformed {
        record: "member",
        reason,
    };
    let instant = |(seconds, nanos)| Timestamp { seconds, nanos };
    Ok(Membership {
        node: NodeId::new(&entity.node).map_err(|_| malformed("node id"))?,
        state: match entity.state.as_str() {
            "approved" => MemberState::Approved,
            "removed" => MemberState::Removed,
            _ => return Err(malformed("state")),
        },
        fingerprint: entity
            .fingerprint
            .map(|text| Fingerprint::new(&text).ok_or_else(|| malformed("fingerprint")))
            .transpose()?,
        token_expires: entity.token_expires.map(instant),
        reason: entity.reason,
        by: entity.by,
        at: instant(entity.at),
    })
}

/// Membership operations.
#[derive(Clone)]
pub struct MembershipService {
    repository: TessariMembers,
}

impl MembershipService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariMembers) -> Self {
        Self { repository }
    }

    /// Every member and tombstone, in node order.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn list(&self) -> Result<Vec<Membership>> {
        self.repository
            .list()
            .await?
            .into_iter()
            .map(membership)
            .collect()
    }

    /// `node`'s record, if any.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn get(&self, node: &NodeId) -> Result<Option<Membership>> {
        self.repository
            .get(node.as_str())
            .await?
            .map(membership)
            .transpose()
    }

    /// Admits `node`, pinned to `fingerprint` when one is given (any certificate naming the node otherwise); any
    /// unused join token for it is withdrawn.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn approve(
        &self,
        node: &NodeId,
        fingerprint: Option<&Fingerprint>,
        by: &str,
        reason: &str,
    ) -> Result<Approval> {
        let grant = Grant {
            node: node.as_str(),
            fingerprint: fingerprint.map(Fingerprint::as_str),
            token: None,
            reason,
            by,
        };
        Ok(if self.repository.grant(&grant).await? {
            Approval::Approved
        } else {
            Approval::Removed
        })
    }

    /// Issues a one-time join token for `node`, lapsing after `lifetime`; until the node offers it, no certificate
    /// is the node's.
    ///
    /// # Errors
    /// [`Error::Randomness`] when the random source fails; the metadata store's refusal or outage.
    pub async fn issue_token(
        &self,
        node: &NodeId,
        lifetime: TokenLifetime,
        by: &str,
        reason: &str,
    ) -> Result<Issued> {
        let token = draw()?;
        let digest = digest(&token);
        let grant = Grant {
            node: node.as_str(),
            fingerprint: None,
            token: Some((&digest, lifetime.secs())),
            reason,
            by,
        };
        if !self.repository.grant(&grant).await? {
            return Ok(Issued::Removed);
        }
        let expires = self
            .get(node)
            .await?
            .and_then(|member| member.token_expires)
            .ok_or(Error::Malformed {
                record: "member",
                reason: "a token was issued and no lapse was stored",
            })?;
        Ok(Issued::Issued { token, expires })
    }

    /// `node` offers `token` with a certificate whose SHA-256 is `fingerprint`: when the token is the node's, unused
    /// and unexpired, the certificate becomes the node's and the token is consumed.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn join(
        &self,
        node: &NodeId,
        token: &JoinToken,
        fingerprint: &Fingerprint,
    ) -> Result<Joined> {
        let bound = self
            .repository
            .bind(node.as_str(), &digest(token), fingerprint.as_str())
            .await?;
        Ok(if bound {
            Joined::Joined
        } else {
            Joined::Refused
        })
    }

    /// Removes `node` for good and deletes its registration, unless `current` still places shards on it.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn remove(
        &self,
        node: &NodeId,
        by: &str,
        reason: &str,
        current: Option<&Layout>,
    ) -> Result<Removal> {
        let placed = current.is_some_and(|layout| {
            layout
                .sets()
                .iter()
                .flatten()
                .any(|member| member.node() == node)
        });
        if placed {
            return Ok(Removal::InLayout);
        }
        let Some(member) = self.get(node).await? else {
            return Ok(Removal::Unknown);
        };
        if member.state == MemberState::Removed
            || !self.repository.remove(node.as_str(), by, reason).await?
        {
            return Ok(Removal::AlreadyRemoved);
        }
        Ok(Removal::Removed)
    }

    /// Whether `node`, presenting a certificate whose SHA-256 is `fingerprint`, is admitted.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn admits(&self, node: &NodeId, fingerprint: &Fingerprint) -> Result<Admission> {
        let Some(entity) = self.repository.get(node.as_str()).await? else {
            return Ok(Admission::Unknown);
        };
        let token_waits = entity.token_waits;
        let member = membership(entity)?;
        Ok(match (member.state, token_waits, member.fingerprint) {
            (MemberState::Removed, _, _) => Admission::Removed,
            (MemberState::Approved, true, _) => Admission::AwaitingToken,
            (MemberState::Approved, false, Some(pinned)) if &pinned != fingerprint => {
                Admission::OtherCertificate
            }
            (MemberState::Approved, false, _) => Admission::Admitted,
        })
    }
}
