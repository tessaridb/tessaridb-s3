//! Object bytes above the inline size: a request body streamed into a data file, and a data file streamed back
//! out block by block. Neither direction holds more than one block (or one body frame) in memory.

use axum::body::{Body, Bytes};
use futures_util::StreamExt;
use tessari_s3_constants::{CHUNKED_FRAMING_ALLOWANCE, STREAMING_MIN_CHUNK_LEN};
use tessari_s3_core::auth::{ChunkedDecoder, PayloadHash, verify_payload_digest};
use tessari_s3_storage::data::DataReader;
use tessari_s3_storage::objects::{ObjectService, Upload, Uploaded};
use tessari_s3_types::ErrorCode;

use crate::pipeline::call::Call;
use crate::{Error, Result};

/// Bytes of aws-chunked framing allowed per chunk: the size line with its signature, and the two line ends.
const FRAMING_PER_CHUNK: u64 = 128;

fn longer_than_declared() -> Error {
    Error::new(
        ErrorCode::IncompleteBody,
        "the body is longer than its declared length",
    )
}

fn incomplete() -> Error {
    Error::new(
        ErrorCode::IncompleteBody,
        "the body ended before its declared length",
    )
}

/// Streams the body (decoded when aws-chunked) into a new data file of exactly `length` bytes and checks the signed
/// payload hash. Every failure here releases the file and its queue entry before it is returned.
pub(super) async fn receive(
    call: &Call<'_>,
    body: Body,
    length: u64,
) -> Result<(Uploaded, Option<String>)> {
    let objects = call.state.storage().objects();
    let mut upload = objects.upload().await?;
    let id = upload.id();
    let outcome = match fill(call, body, length, &mut upload).await {
        Ok(trailer) => match upload.finish().await {
            Ok(uploaded) => match call.verified.payload() {
                PayloadHash::Sha256(expected) => {
                    verify_payload_digest(&expected, &uploaded.digests.sha256)
                        .map(|()| (uploaded, trailer))
                        .map_err(Error::from)
                }
                _ => Ok((uploaded, trailer)),
            },
            Err(error) => Err(error.into()),
        },
        Err(error) => {
            drop(upload);
            Err(error)
        }
    };
    if outcome.is_err() {
        release(objects, id).await;
    }
    outcome
}

/// Releases data `id` after a write that will not commit; a failure leaves it queued for the reclaimer.
pub(super) async fn release(objects: &ObjectService, id: [u8; 16]) {
    if let Err(error) = objects.release(id).await {
        tracing::warn!(error = %error, "an uncommitted data file was left for the reclaimer");
    }
}

/// Copies the body into `upload`, refusing a body longer or shorter than `length`; answers the trailing checksum.
async fn fill(
    call: &Call<'_>,
    body: Body,
    length: u64,
    upload: &mut Upload,
) -> Result<Option<String>> {
    let mut frames = body.into_data_stream();
    match call.verified.payload() {
        PayloadHash::StreamingSigned
        | PayloadHash::StreamingSignedTrailer
        | PayloadHash::StreamingUnsignedTrailer => {
            let trailer_header = call
                .headers
                .get("x-amz-trailer")
                .and_then(|v| v.to_str().ok());
            let mut decoder = ChunkedDecoder::new(call.verified.clone(), length, trailer_header)?;
            let raw_limit = (length / STREAMING_MIN_CHUNK_LEN)
                .checked_add(2)
                .and_then(|chunks| chunks.checked_mul(FRAMING_PER_CHUNK))
                .and_then(|framing| framing.checked_add(length))
                .and_then(|limit| limit.checked_add(len_of(CHUNKED_FRAMING_ALLOWANCE)))
                .ok_or_else(longer_than_declared)?;
            let (mut raw, mut decoded) = (0_u64, 0_u64);
            let mut out = Vec::new();
            while let Some(frame) = frames.next().await {
                let frame = frame.map_err(|_| incomplete())?;
                raw = raw.saturating_add(len(&frame));
                if raw > raw_limit {
                    return Err(longer_than_declared());
                }
                decoder.push(&frame, &mut out)?;
                decoded = decoded.saturating_add(len(&out));
                if decoded > length {
                    return Err(longer_than_declared());
                }
                upload.append(&out).await?;
                out.clear();
            }
            Ok(decoder.finish()?.map(|trailer| trailer.value))
        }
        PayloadHash::Sha256(_) | PayloadHash::Unsigned => {
            let mut total = 0_u64;
            while let Some(frame) = frames.next().await {
                let frame = frame.map_err(|_| incomplete())?;
                total = total.saturating_add(len(&frame));
                if total > length {
                    return Err(longer_than_declared());
                }
                upload.append(&frame).await?;
            }
            if total == length {
                Ok(None)
            } else {
                Err(incomplete())
            }
        }
    }
}

fn len(bytes: &[u8]) -> u64 {
    len_of(bytes.len())
}

/// A length as a byte count; a `usize` beyond `u64` (not a platform this runs on) saturates, which every caller
/// treats as "too long".
fn len_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// Where one block's part of the range `start..=end` lies inside the block.
fn part(block: &[u8], block_start: u64, start: u64, end: u64) -> Option<(usize, usize)> {
    let from = usize::try_from(start.saturating_sub(block_start)).ok()?;
    let last_in_block = block_start.checked_add(len(block))?.checked_sub(1)?;
    let to = usize::try_from(end.min(last_in_block).checked_sub(block_start)?).ok()?;
    (from <= to && to < block.len()).then_some((from, to))
}

/// What the streaming body still has to send.
struct Remaining {
    reader: DataReader,
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
pub(super) async fn send(
    objects: &ObjectService,
    id: [u8; 16],
    size: u64,
    start: u64,
    end: u64,
) -> Result<Body> {
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
        Error::new(
            ErrorCode::InternalError,
            "we encountered an internal error; please try again",
        )
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
    let stream =
        futures_util::stream::once(async move { Ok::<_, std::io::Error>(head) }).chain(rest);
    Ok(Body::from_stream(stream))
}
