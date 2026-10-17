//! Re-wrapping after a rotation (G017 W6): a data key wrapped under an older server key is unwrapped and wrapped
//! again under the active one, in its record; the data file it seals is not touched. Rotation is complete when no
//! record names the old key.

use super::super::model::Rewrapped;
use super::super::tessaridb::rewrap::Holder;
use super::ObjectService;
use crate::Result;
use crate::encryption::{unwrap, wrap};

impl ObjectService {
    /// Re-wraps, looking at most at `limit` records, the data keys wrapped under a server key older than this node's
    /// active one.
    ///
    /// # Errors
    /// [`crate::Error::KeysSealed`] when this node holds no keys; a record whose key does not unwrap; the metadata
    /// store's refusal or outage.
    pub async fn rewrap(&self, limit: usize) -> Result<Rewrapped> {
        let ring = self.keys.ring()?;
        let mut done = Rewrapped::default();
        for old in ring.behind() {
            for holder in Holder::ALL {
                let left = limit.saturating_sub(done.examined);
                if left == 0 {
                    return Ok(done);
                }
                for record in self
                    .repository
                    .wrapped_under(holder, old.id(), left)
                    .await?
                {
                    done.examined = done.examined.saturating_add(1);
                    let key = unwrap(old, &record.wrapped, record.data)?;
                    let fresh = wrap(ring.active(), &key, record.data)?;
                    if self.repository.rewrap(&record, &fresh).await? {
                        done.rewrapped = done.rewrapped.saturating_add(1);
                    }
                }
            }
        }
        Ok(done)
    }

    /// How many records still have their data key wrapped under `kek_id`.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn remaining(&self, kek_id: &str) -> Result<u64> {
        let mut total: u64 = 0;
        for holder in Holder::ALL {
            let counted = self.repository.count_wrapped_under(holder, kek_id).await?;
            total = total.saturating_add(counted);
        }
        Ok(total)
    }
}
