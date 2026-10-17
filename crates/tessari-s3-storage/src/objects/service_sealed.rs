//! The sealed side of an object's data: writing and opening bytes sealed under a data key of their own, wrapped under
//! the server's key, a named key, or a key derived from a customer's (SSE-C).

use super::ObjectService;
use crate::encryption::stream::{Sealer, sealed_size};
use crate::encryption::{
    CustomerKey, CustomerSeal, DataKey, KeyEncryptionKey, Sealing, WrappedKey, unwrap, wrap,
};
use crate::objects::ObjectReader;
use crate::objects::upload::Upload;
use crate::{Error, Result};

impl ObjectService {
    /// As [`Self::upload`], every byte sealed under a data key drawn for this data id alone and wrapped under the
    /// key `sealing` names — the server's active key, or a named one; the finished upload carries the wrapped key. On
    /// a cluster member the shards are sealed shards (format 4).
    ///
    /// # Errors
    /// [`Error::KeysSealed`] when this node holds no encryption keys, [`Error::UnknownKey`] for a name it does not
    /// hold; as [`Self::upload`]; [`Error::SealedLength`] when `size` cannot be laid out in sealed blocks.
    pub async fn upload_sealed(&self, size: u64, sealing: &Sealing) -> Result<Upload> {
        let ring = self.keys.ring()?;
        let kek = match sealing {
            Sealing::Server => ring.active(),
            Sealing::Named(name) => {
                ring.get(&name.record_id())
                    .ok_or_else(|| Error::UnknownKey {
                        name: name.to_string(),
                    })?
            }
        };
        self.upload_under(size, kek, None).await
    }

    /// As [`Self::upload_sealed`], its data key wrapped under a key derived from the customer's `key` and `seal` (SSE-C);
    /// the finished upload carries the seal, never the key.
    ///
    /// # Errors
    /// [`Error::CustomerKeyMismatch`] when `key` is not the one `seal` was made for; as [`Self::upload_sealed`].
    pub async fn upload_customer(
        &self,
        size: u64,
        key: &CustomerKey,
        seal: &CustomerSeal,
    ) -> Result<Upload> {
        if !seal.matches(key)? {
            return Err(Error::CustomerKeyMismatch);
        }
        self.upload_under(size, &seal.kek(key)?, Some(seal.clone()))
            .await
    }

    /// Starts writing `size` bytes sealed under a fresh data key wrapped under `kek`, carrying `customer` when the
    /// wrapping key came from a customer's key.
    async fn upload_under(
        &self,
        size: u64,
        kek: &KeyEncryptionKey,
        customer: Option<CustomerSeal>,
    ) -> Result<Upload> {
        let id = *uuid::Uuid::new_v4().as_bytes();
        let stored_block = self.stored_block();
        let stored = sealed_size(size, stored_block).ok_or(Error::SealedLength {
            declared: size,
            received: 0,
        })?;
        let key = DataKey::generate()?;
        let mut wrapped = wrap(kek, &key, id)?;
        wrapped.customer = customer;
        let sealer = Sealer::new(key, id, size, stored_block)?;
        Ok(self.start(id, stored, true).await?.sealing(sealer, wrapped))
    }

    /// As [`Self::open`], for data sealed under the data key `wrapped` holds, which an encryption key of this node's
    /// unwraps.
    ///
    /// # Errors
    /// [`Error::KeysSealed`] when this node holds no encryption keys; as [`Self::open`]; [`Error::Unsealable`] when
    /// no key held unwraps it, when the stored bytes do not lay out `size`, or per block when one does not open.
    pub async fn open_sealed(
        &self,
        id: [u8; 16],
        size: u64,
        wrapped: &WrappedKey,
    ) -> Result<ObjectReader> {
        let key = {
            let ring = self.keys.ring()?;
            let kek = ring.get(&wrapped.kek_id).ok_or(Error::Unsealable)?;
            unwrap(kek, wrapped, id)?
        };
        self.open_with(id, size, key).await
    }

    /// As [`Self::open_sealed`], for data sealed under a customer's key (SSE-C): `key` must be the one the wrapped key's
    /// seal was made for.
    ///
    /// # Errors
    /// [`Error::CustomerKeyMismatch`] for another key; [`Error::Unsealable`] for a wrapped key with no customer seal;
    /// as [`Self::open`].
    pub async fn open_customer(
        &self,
        id: [u8; 16],
        size: u64,
        wrapped: &WrappedKey,
        key: &CustomerKey,
    ) -> Result<ObjectReader> {
        let seal = wrapped.customer.as_ref().ok_or(Error::Unsealable)?;
        if !seal.matches(key)? {
            return Err(Error::CustomerKeyMismatch);
        }
        let key = unwrap(&seal.kek(key)?, wrapped, id)?;
        self.open_with(id, size, key).await
    }

    /// Opens data `id` holding an object of `size` bytes sealed under `key`.
    async fn open_with(&self, id: [u8; 16], size: u64, key: DataKey) -> Result<ObjectReader> {
        let stored = sealed_size(size, self.stored_block()).ok_or(Error::Unsealable)?;
        let reader = self.open_stored(id, stored, true).await?;
        ObjectReader::sealed(reader, key, id, size)
    }
}
