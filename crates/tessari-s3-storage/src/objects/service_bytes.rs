//! Objects the server reads and writes whole, for its own work: an original a media worker renders from, held in
//! memory under a bound, and a rendition it writes back as an ordinary object.

use std::collections::BTreeMap;

use tessari_s3_core::objects::checksum::ChecksumAlgorithm;
use tessari_s3_types::{BucketName, ObjectKey};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::ObjectService;
use crate::objects::{
    Content, Lock, NewObject, StoredObject, Upload, VersionRead, WriteCondition, Written,
};
use crate::{Error, Result};

impl ObjectService {
    /// The object `version_id` names at `bucket/key`, or the current one when none is named; none when it is gone, a
    /// delete marker, or its bucket is.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn stored(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version_id: Option<&str>,
    ) -> Result<Option<StoredObject>> {
        Ok(match version_id {
            Some(version) => match self.get_version(bucket, key, version).await? {
                VersionRead::Object(object) => Some(object),
                _ => None,
            },
            None => self.get(bucket, key).await?.ok().flatten(),
        })
    }

    /// Every byte of `object`, or none when it is larger than `max` bytes; a multipart object is read part by part.
    ///
    /// # Errors
    /// A data file's or the shards' failure.
    pub async fn read_whole(&self, object: &StoredObject, max: u64) -> Result<Option<Vec<u8>>> {
        let mut bytes = Vec::with_capacity(usize::try_from(object.size.min(max)).unwrap_or(0));
        Ok(self
            .copy_to(object, max, &mut bytes)
            .await?
            .then_some(bytes))
    }

    /// Writes every byte of `object` to `out`, block by block; `false`, having written nothing, when it is larger than
    /// `max` bytes.
    ///
    /// # Errors
    /// A data file's or the shards' failure, or `out`'s.
    pub async fn copy_to(
        &self,
        object: &StoredObject,
        max: u64,
        out: &mut (impl AsyncWrite + Unpin + Send),
    ) -> Result<bool> {
        if object.size > max {
            return Ok(false);
        }
        let pieces = match &object.content {
            Content::Data(id) => vec![(*id, object.size)],
            Content::Parts(multipart) => multipart
                .parts
                .iter()
                .map(|part| (part.data, part.size))
                .collect(),
        };
        for (id, size) in pieces {
            let reader = self.open(id, size).await?;
            let block = u64::from(reader.block_size()).max(1);
            for index in 0..size.div_ceil(block) {
                out.write_all(&reader.read_block(index).await?)
                    .await
                    .map_err(|error| Error::DataIo {
                        kind: error.kind(),
                        op: "copy out",
                    })?;
            }
        }
        out.flush().await.map_err(|error| Error::DataIo {
            kind: error.kind(),
            op: "copy out",
        })?;
        Ok(true)
    }

    /// Writes `bytes` at `bucket/key` as an object of `content_type`, over whatever is there, with an MD5 ETag and the
    /// default checksum as a PUT of the same bytes would have. A write that does not commit releases its file.
    ///
    /// # Errors
    /// The drive's failure, or the metadata store's refusal or outage.
    pub async fn put_bytes(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<Written> {
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let mut source = bytes;
        self.put_reader(bucket, key, &mut source, length, content_type)
            .await
    }

    /// As [`Self::put_bytes`], the `length` bytes read from `source`; a source that ends early or runs long is refused
    /// and its file released.
    ///
    /// # Errors
    /// The drive's or `source`'s failure, or the metadata store's refusal or outage.
    pub async fn put_reader(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        source: &mut (impl AsyncRead + Unpin + Send),
        length: u64,
        content_type: &str,
    ) -> Result<Written> {
        let mut upload = self.upload(length).await?;
        let id = upload.id();
        let uploaded = match fill(&mut upload, source, length).await {
            Ok(()) => upload.finish().await,
            Err(error) => Err(error),
        };
        let uploaded = match uploaded {
            Ok(uploaded) => uploaded,
            Err(error) => {
                self.release_logged(id).await;
                return Err(error);
            }
        };
        let algorithm = ChecksumAlgorithm::Crc64Nvme;
        let object = NewObject {
            size: uploaded.size,
            etag: format!("\"{}\"", uploaded.digests.md5_hex),
            headers: BTreeMap::from([("content-type".to_owned(), content_type.to_owned())]),
            metadata: BTreeMap::new(),
            checksums: BTreeMap::from([(
                algorithm.name().to_owned(),
                uploaded.digests.value(algorithm).to_owned(),
            )]),
            content: Content::Data(uploaded.id),
            copied: false,
            lock: Lock::default(),
        };
        self.put(bucket, key, &object, &WriteCondition::None).await
    }
}

/// Bytes copied into an upload per read.
const COPY_CHUNK: usize = 1024 * 1024;

/// Copies exactly `length` bytes of `source` into `upload`.
async fn fill(
    upload: &mut Upload,
    source: &mut (impl AsyncRead + Unpin + Send),
    length: u64,
) -> Result<()> {
    let io = |error: std::io::Error| Error::DataIo {
        kind: error.kind(),
        op: "read",
    };
    let mut chunk = vec![0_u8; COPY_CHUNK];
    let mut copied = 0_u64;
    loop {
        let read = source.read(&mut chunk).await.map_err(io)?;
        if read == 0 {
            break;
        }
        copied = copied.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if copied > length {
            return Err(io(std::io::ErrorKind::InvalidData.into()));
        }
        upload.append(&chunk[..read]).await?;
    }
    if copied == length {
        Ok(())
    } else {
        Err(io(std::io::ErrorKind::UnexpectedEof.into()))
    }
}
