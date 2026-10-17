//! A replica's request body: the stored pieces of a version, block by block, each opened under its own key.

use std::io;

use bytes::Bytes;
use futures_util::Stream;
use tessari_s3_storage::objects::{ObjectReader, ObjectService};

use super::Piece;

/// Where a body stream stands: the pieces left, and the piece open now with its next block and block count.
struct Blocks {
    objects: ObjectService,
    pieces: std::vec::IntoIter<Piece>,
    open: Option<(ObjectReader, u64, u64)>,
}

/// The bytes of `pieces`, block by block and opened under their own keys, as a request body; an unreadable block ends
/// it.
pub(super) fn blocks(
    objects: ObjectService,
    pieces: Vec<Piece>,
) -> impl Stream<Item = std::result::Result<Bytes, io::Error>> + Send + 'static {
    let start = Blocks {
        objects,
        pieces: pieces.into_iter(),
        open: None,
    };
    futures_util::stream::unfold(start, |mut at| async move {
        loop {
            if let Some((reader, index, count)) = at.open.take()
                && index < count
            {
                let block = reader
                    .read_block(index)
                    .await
                    .map(Bytes::from)
                    .map_err(|error| io::Error::other(error.to_string()));
                if block.is_err() {
                    at.pieces = Vec::new().into_iter();
                } else {
                    at.open = Some((reader, index.saturating_add(1), count));
                }
                return Some((block, at));
            }
            let (id, size, sealed) = at.pieces.next()?;
            let opened = match &sealed {
                Some(wrapped) => at.objects.open_sealed(id, size, wrapped).await,
                None => at.objects.open(id, size).await,
            };
            match opened {
                Ok(reader) => {
                    let count = size.div_ceil(u64::from(reader.block_size()).max(1));
                    at.open = Some((reader, 0, count));
                }
                Err(error) => {
                    at.pieces = Vec::new().into_iter();
                    return Some((Err(io::Error::other(error.to_string())), at));
                }
            }
        }
    })
}
