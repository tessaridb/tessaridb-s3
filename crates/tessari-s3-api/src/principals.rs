//! Resolving a user's access key to the user as the access evaluator sees it and the secret its requests are signed
//! with, behind a cache shared by every request of this node.
//!
//! The cache holds the decision's INPUTS (principal and secret), never a decision: each request is still judged by
//! `authorize`. An entry is believed for [`WINDOW_SECS`] after it was resolved, so disabling a user or a key, or
//! changing a grant, binds on every node within that window. Only live keys are cached: an unknown or disabled key
//! is looked up again on every request (one read by identity, the cost a random id costs anyway), so random ids
//! cannot grow the map and a newly issued key is never held back by a cached refusal. A lookup that fails fails the
//! request: nothing is served past its window.

use std::future::Future;
use std::sync::Arc;

use scc::TreeIndex;
use tessari_s3_core::authz::UserPrincipal;
use tessari_s3_storage::users::UserService;
use tessari_s3_types::SecretKey;

/// How long a resolution is believed, in seconds: the published revocation window.
pub(crate) const WINDOW_SECS: i64 = 5;

/// What looking a key up in the store answered.
pub(crate) enum Fetched {
    /// The key and its user are enabled.
    Live {
        /// The user, with its grants.
        principal: UserPrincipal,
        /// The key's secret.
        secret: SecretKey,
    },
    /// No such key, or it or its user is disabled.
    Unknown,
}

/// What a key resolves to: who signs with it and the secret to check the signature with, or nothing.
#[derive(Clone)]
pub(crate) struct Live {
    /// The user, with its grants.
    pub(crate) principal: UserPrincipal,
    /// The key's secret.
    pub(crate) secret: SecretKey,
}

/// One cached resolution of a live key.
struct Entry {
    resolved_at: i64,
    live: Live,
}

/// The cache, keyed by access key id.
pub(crate) struct Principals {
    entries: TreeIndex<String, Arc<Entry>>,
}

impl Principals {
    /// An empty cache.
    pub(crate) fn new() -> Self {
        Self {
            entries: TreeIndex::new(),
        }
    }

    /// The resolution of key `id` at `now` (seconds): the cached one while it is younger than the window, otherwise
    /// what `fetch` answers, which then replaces it.
    ///
    /// # Errors
    /// Whatever `fetch` fails with; nothing stale is served in its place.
    pub(crate) async fn resolve<E, F, Fut>(
        &self,
        id: &str,
        now: i64,
        fetch: F,
    ) -> Result<Option<Live>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Fetched, E>>,
    {
        // Copy the entry out: no Guard outlives this line, let alone an `.await`.
        let cached = self.entries.peek_with(id, |_, entry| Arc::clone(entry));
        if let Some(entry) = cached {
            let fresh = now
                .checked_sub(entry.resolved_at)
                .is_some_and(|age| (0..WINDOW_SECS).contains(&age));
            if fresh {
                return Ok(Some(entry.live.clone()));
            }
        }
        let Fetched::Live { principal, secret } = fetch().await? else {
            self.entries.remove_async(id).await;
            return Ok(None);
        };
        let live = Live { principal, secret };
        let entry = Arc::new(Entry {
            resolved_at: now,
            live: live.clone(),
        });
        self.entries.upsert_async(id.to_owned(), entry).await;
        Ok(Some(live))
    }
}

/// User key `id` resolved through `cache` at `now`: its user, with grants, and its secret while both are enabled.
///
/// # Errors
/// The store's failure; nothing cached past its window is served in its place.
pub(crate) async fn resolve_user_key(
    cache: &Principals,
    users: &UserService,
    id: &str,
    now: i64,
) -> Result<Option<Live>, tessari_s3_storage::Error> {
    cache
        .resolve(id, now, || async {
            let Some(resolved) = users.resolve(id).await? else {
                return Ok(Fetched::Unknown);
            };
            Ok(match users.principal(&resolved.user.name).await? {
                Some(principal) => Fetched::Live {
                    principal,
                    secret: resolved.secret,
                },
                None => Fetched::Unknown,
            })
        })
        .await
}

#[cfg(test)]
#[path = "principals_tests.rs"]
mod tests;
