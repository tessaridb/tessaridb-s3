//! Object bytes above the inline size: a request body streamed into a data file without holding more than one body
//! frame in memory. Streaming the stored bytes back out is [`super::body`].

use axum::body::Body;
use futures_util::StreamExt;
use tessari_s3_constants::{CHUNKED_FRAMING_ALLOWANCE, STREAMING_MIN_CHUNK_LEN};
use tessari_s3_core::auth::{ChunkedDecoder, PayloadHash, verify_payload_digest};
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
pub(crate) async fn receive(
    call: &Call<'_>,
    body: Body,
    length: u64,
) -> Result<(Uploaded, Option<String>)> {
    let objects = call.state.storage().objects();
    let mut upload = objects.upload(length).await?;
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
pub(crate) async fn release(objects: &ObjectService, id: [u8; 16]) {
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

pub(super) fn len(bytes: &[u8]) -> u64 {
    len_of(bytes.len())
}

/// A length as a byte count; a `usize` beyond `u64` (not a platform this runs on) saturates, which every caller
/// treats as "too long".
fn len_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}
