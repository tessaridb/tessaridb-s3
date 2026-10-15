//! Stored object bytes streamed back out block by block, whether they sit in one data file or in erasure-coded shards;
//! no more than one block is held in memory.

use axum::body::{Body, Bytes};
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use tessari_s3_core::objects::range::segments;
use tessari_s3_storage::Error as StorageError;
use tessari_s3_storage::objects::{ObjectReader, ObjectService};
use tessari_s3_types::ErrorCode;

use super::data::len;
use crate::{Error, Result};

/// Where one block's part of the range `start..=end` lies inside the block.
fn part(block: &[u8], block_start: u64, start: u64, end: u64) -> Option<(usize, usize)> {
    let from = usize::try_from(start.saturating_sub(block_start)).ok()?;
    let last_in_block = block_start.checked_add(len(block))?.checked_sub(1)?;
    let to = usize::try_from(end.min(last_in_block).checked_sub(block_start)?).ok()?;
    (from <= to && to < block.len()).then_some((from, to))
}

/// What the streaming body still has to send.
struct Remaining {
    reader: ObjectReader,
    next: u64,
    last: u64,
    start: u64,
    end: u64,
}

impl Remaining {
    fn block_start(&self, index: u64) -> Option<u64> {
        index.checked_mul(u64::from(self.reader.block_size()))
    }

    /// Block `index`'s share of the range, read and verified.
    async fn read(&self, index: u64) -> std::io::Result<Bytes> {
        let block = self
            .reader
            .read_block(index)
            .await
            .map_err(std::io::Error::other)?;
        let block_start = self
            .block_start(index)
            .ok_or_else(|| std::io::Error::other("block offset overflows"))?;
        let (from, to) = part(&block, block_start, self.start, self.end)
            .ok_or_else(|| std::io::Error::other("block outside the range"))?;
        Ok(Bytes::copy_from_slice(&block[from..=to]))
    }
}

/// The body for bytes `start..=end` of data `id` holding `size` bytes. The first block is read and verified before
/// this returns, so damage there becomes an error response; damage in a later block ends the body early, which a
/// client sees as a body shorter than its Content-Length.
pub(crate) async fn send(
    objects: &ObjectService,
    id: [u8; 16],
    size: u64,
    start: u64,
    end: u64,
) -> Result<Body> {
    Ok(Body::from_stream(
        blocks(objects, id, size, start, end).await?,
    ))
}

/// The body for bytes `start..=end` of an object made of `parts` — `(data id, size)` in object order. The first part's
/// first block is verified before this returns, as for one data file; each later part's file is opened when the body
/// reaches it, and a failure there ends the body early.
pub(crate) async fn send_parts(
    objects: &ObjectService,
    parts: &[([u8; 16], u64)],
    start: u64,
    end: u64,
) -> Result<Body> {
    Ok(Body::from_stream(
        stream_parts(objects, parts, start, end).await?,
    ))
}

/// The verified bytes `start..=end` of an object made of `parts`, as [`send_parts`] sends them.
pub(super) async fn stream_parts(
    objects: &ObjectService,
    parts: &[([u8; 16], u64)],
    start: u64,
    end: u64,
) -> Result<BoxStream<'static, std::io::Result<Bytes>>> {
    let sizes: Vec<u64> = parts.iter().map(|(_, size)| *size).collect();
    let mut wanted = segments(&sizes, start, end)
        .into_iter()
        .filter_map(|segment| {
            parts
                .get(segment.index)
                .map(|(id, size)| (*id, *size, segment.start, segment.end))
        });
    let Some((id, size, from, to)) = wanted.next() else {
        return Ok(futures_util::stream::empty().boxed());
    };
    let first = blocks(objects, id, size, from, to).await?;
    let objects = objects.clone();
    let rest: Vec<([u8; 16], u64, u64, u64)> = wanted.collect();
    let later = futures_util::stream::iter(rest)
        .then(move |(id, size, from, to)| {
            let objects = objects.clone();
            async move { blocks(&objects, id, size, from, to).await }
        })
        .flat_map(|opened| match opened {
            Ok(stream) => stream,
            Err(error) => {
                tracing::error!(error = %error, "a part file of a multipart object could not be opened");
                futures_util::stream::once(async move { Err(std::io::Error::other(error.message)) }).boxed()
            }
        });
    Ok(first.chain(later).boxed())
}

/// The verified blocks holding bytes `start..=end` of data `id`, the first already read.
pub(super) async fn blocks(
    objects: &ObjectService,
    id: [u8; 16],
    size: u64,
    start: u64,
    end: u64,
) -> Result<BoxStream<'static, std::io::Result<Bytes>>> {
    let reader = objects.open(id, size).await?;
    let block_size = u64::from(reader.block_size());
    let block_of = |offset: u64| {
        offset.checked_div(block_size).ok_or_else(|| {
            Error::new(
                ErrorCode::InternalError,
                "a data file declares no block size",
            )
        })
    };
    let (first, last) = (block_of(start)?, block_of(end)?);
    let remaining = Remaining {
        reader,
        next: first,
        last,
        start,
        end,
    };
    let head = remaining.read(first).await.map_err(|error| {
        tracing::error!(error = %error, "the first block of a data object failed");
        // The storage failure keeps its own answer (too few shards is 503); anything else is internal.
        match error
            .into_inner()
            .map(|inner| inner.downcast::<StorageError>())
        {
            Some(Ok(storage)) => Error::from(*storage),
            _ => Error::new(
                ErrorCode::InternalError,
                "we encountered an internal error; please try again",
            ),
        }
    })?;
    let rest = futures_util::stream::unfold(Some(remaining), |state| async move {
        let mut remaining = state?;
        remaining.next = remaining.next.checked_add(1)?;
        if remaining.next > remaining.last {
            return None;
        }
        match remaining.read(remaining.next).await {
            Ok(bytes) => Some((Ok(bytes), Some(remaining))),
            Err(error) => {
                tracing::error!(error = %error, "a data object's body ended at a failed block");
                Some((Err(error), None))
            }
        }
    });
    Ok(
        futures_util::stream::once(async move { Ok::<_, std::io::Error>(head) })
            .chain(rest)
            .boxed(),
    )
}
