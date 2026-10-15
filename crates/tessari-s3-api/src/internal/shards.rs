//! The shard routes. A PUT streams its body into the shard file one bounded block at a time and answers 200 only
//! once the file and its directory are synced; a GET streams verified blocks; a DELETE removes the shard.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use tessari_s3_storage::shards::ShardWrite;

use super::InternalState;
use super::params::{Params, data_id, index};
use super::refusal::Refusal;

/// `PUT {path}/{id}/{index}?block=&size=` — stores the body as the shard.
pub(super) async fn put_shard(
    State(state): State<InternalState>,
    Path((id, shard)): Path<(String, String)>,
    uri: Uri,
    body: Body,
) -> Response {
    let shard = async {
        let (id, shard) = (data_id(&id)?, index(&shard)?);
        let params = Params::parse(uri.query(), &["block", "size"])?;
        let (block, size) = (params.required("block")?, params.required("size")?);
        let mut write = state
            .storage()
            .shards()
            .write(id, shard, block, size)
            .await?;
        let mut body = body.into_data_stream();
        while let Some(chunk) = body.next().await {
            let appended = match chunk {
                Ok(bytes) => write.append(&bytes).await.map_err(Refusal::from),
                Err(_) => Err(Refusal::Invalid("the body ended early")),
            };
            if let Err(refusal) = appended {
                abandon(write).await;
                return Err(refusal);
            }
        }
        write.finish().await.map_err(Refusal::from)
    };
    match shard.await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

/// Removes what a refused write left; a failure here leaves a temporary file the next write of the shard is refused
/// over, which is logged rather than hidden behind the refusal the caller already gets.
async fn abandon(write: ShardWrite) {
    if let Err(error) = write.abandon().await {
        tracing::error!(error = %error, "a refused shard write could not be removed");
    }
}

/// `GET {path}/{id}/{index}?size=[&from=&to=]` — the shard's blocks `[from, to)`, each verified before it is sent.
/// A block failing verification part way ends the body early; the caller knows the length it asked for.
pub(super) async fn get_shard(
    State(state): State<InternalState>,
    Path((id, shard)): Path<(String, String)>,
    uri: Uri,
) -> Response {
    let blocks = async {
        let (id, shard) = (data_id(&id)?, index(&shard)?);
        let params = Params::parse(uri.query(), &["size", "from", "to"])?;
        let size: u64 = params.required("size")?;
        let reader = state.storage().shards().open(id, shard, size).await?;
        let count = size.div_ceil(u64::from(reader.block_size()));
        let from = params.optional("from")?.unwrap_or(0);
        let to = params.optional("to")?.unwrap_or(count);
        if from > to || to > count {
            return Err(Refusal::Invalid("the block range is not within the shard"));
        }
        Ok((Arc::new(reader), from, to))
    };
    let (reader, from, to) = match blocks.await {
        Ok(found) => found,
        Err(refusal) => return refusal.into_response(),
    };
    let stream = futures_util::stream::try_unfold(from, move |block| {
        let reader = Arc::clone(&reader);
        async move {
            if block >= to {
                return Ok(None);
            }
            let bytes = reader.read_block(block).await.inspect_err(|error| {
                tracing::error!(block, error = %error, "a shard block could not be sent");
            })?;
            Ok::<_, tessari_s3_storage::Error>(Some((Bytes::from(bytes), block.saturating_add(1))))
        }
    });
    (StatusCode::OK, Body::from_stream(stream)).into_response()
}

/// `DELETE {path}/{id}/{index}` — removes the shard; a missing shard is not an error.
pub(super) async fn delete_shard(
    State(state): State<InternalState>,
    Path((id, shard)): Path<(String, String)>,
    uri: Uri,
) -> Response {
    let removed = async {
        let (id, shard) = (data_id(&id)?, index(&shard)?);
        Params::parse(uri.query(), &[])?;
        state.storage().shards().remove(id, shard).await?;
        Ok::<_, Refusal>(())
    };
    match removed.await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(refusal) => refusal.into_response(),
    }
}
