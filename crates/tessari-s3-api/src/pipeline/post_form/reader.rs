//! Reading a `multipart/form-data` upload: the fields before the file, bounded, and then the file itself as a stream
//! of exactly the length the request leaves for it — so it is never held whole — whose closing delimiter is checked
//! before its last byte is handed on.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::{Body, BodyDataStream, Bytes};
use futures_util::StreamExt;
use tessari_s3_types::ErrorCode;

use crate::{Error, Result};

/// The most the fields before the file may take, headers included.
const FIELDS_MAX: u64 = 20 * 1024;

pub(super) fn malformed(message: &str) -> Error {
    Error::new(ErrorCode::MalformedPostRequest, message.to_owned())
}

/// A form read up to its file.
pub(super) struct Form {
    /// The fields in order, names lowercased.
    pub(super) fields: Vec<(String, String)>,
    /// The file part's name as the browser gave it.
    pub(super) filename: String,
    /// The file's bytes, `length` of them.
    pub(super) file: Body,
    /// The file's length.
    pub(super) length: u64,
    /// Set when the body did not end as the length promised: the upload is malformed, not failed.
    pub(super) broken: Arc<AtomicBool>,
}

struct Reader {
    stream: BodyDataStream,
    buf: Vec<u8>,
    consumed: u64,
}

impl Reader {
    async fn fill(&mut self) -> Result<bool> {
        match self.stream.next().await {
            Some(Ok(bytes)) => {
                self.buf.extend_from_slice(&bytes);
                Ok(true)
            }
            Some(Err(_)) => Err(malformed("the form could not be read")),
            None => Ok(false),
        }
    }

    /// The bytes before the next `delimiter`, which is consumed too; refused once the fields grow past their bound.
    async fn until(&mut self, delimiter: &[u8]) -> Result<Vec<u8>> {
        loop {
            if let Some(at) = self
                .buf
                .windows(delimiter.len())
                .position(|window| window == delimiter)
            {
                let before = self.buf[..at].to_vec();
                let taken = at.saturating_add(delimiter.len());
                self.buf.drain(..taken);
                self.consumed = self
                    .consumed
                    .saturating_add(u64::try_from(taken).unwrap_or(u64::MAX));
                if self.consumed > FIELDS_MAX {
                    return Err(malformed("the form's fields exceed 20 KB"));
                }
                return Ok(before);
            }
            if u64::try_from(self.buf.len()).unwrap_or(u64::MAX) > FIELDS_MAX {
                return Err(malformed("the form's fields exceed 20 KB"));
            }
            if !self.fill().await? {
                return Err(malformed("the form ends before its file"));
            }
        }
    }

    /// The next `count` bytes.
    async fn take(&mut self, count: usize) -> Result<Vec<u8>> {
        while self.buf.len() < count {
            if !self.fill().await? {
                return Err(malformed("the form ends before its file"));
            }
        }
        self.consumed = self
            .consumed
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        Ok(self.buf.drain(..count).collect())
    }
}

/// `name="…"` from a Content-Disposition header.
fn parameter(disposition: &str, name: &str) -> Option<String> {
    disposition.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().trim_matches('"').to_owned())
    })
}

/// The boundary a `multipart/form-data` content type names.
pub(super) fn boundary(content_type: &str) -> Option<String> {
    let (kind, rest) = content_type.split_once(';')?;
    if !kind.trim().eq_ignore_ascii_case("multipart/form-data") {
        return None;
    }
    parameter(rest, "boundary").filter(|boundary| !boundary.is_empty() && boundary.len() <= 70)
}

/// Reads `body` up to the file part, whose length is `content_length` less what came before it and the closing
/// delimiter.
///
/// # Errors
/// `MalformedPOSTRequest` for a form this does not read, one with no file, or a length that cannot hold one.
pub(super) async fn read(body: Body, boundary: &str, content_length: u64) -> Result<Form> {
    let mut reader = Reader {
        stream: body.into_data_stream(),
        buf: Vec::new(),
        consumed: 0,
    };
    let open = format!("--{boundary}");
    let next = format!("\r\n--{boundary}");
    reader.until(open.as_bytes()).await?;
    if reader.take(2).await? != b"\r\n" {
        return Err(malformed("the form's first boundary is malformed"));
    }
    let mut fields = Vec::new();
    loop {
        let head = String::from_utf8(reader.until(b"\r\n\r\n").await?)
            .map_err(|_| malformed("a part's headers are not text"))?;
        let disposition = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("content-disposition")
                    .then_some(value)
            })
            .ok_or_else(|| malformed("a part has no Content-Disposition"))?;
        let name = parameter(disposition, "name")
            .ok_or_else(|| malformed("a part has no name"))?
            .to_ascii_lowercase();
        if name == "file" {
            let filename = parameter(disposition, "filename").unwrap_or_default();
            let closing = format!("{next}--\r\n").into_bytes();
            let length = content_length
                .checked_sub(reader.consumed)
                .and_then(|left| left.checked_sub(u64::try_from(closing.len()).ok()?))
                .ok_or_else(|| malformed("the form's length leaves no room for its file"))?;
            let broken = Arc::new(AtomicBool::new(false));
            let file = file_body(reader, length, closing, Arc::clone(&broken));
            return Ok(Form {
                fields,
                filename,
                file,
                length,
                broken,
            });
        }
        let value = String::from_utf8(reader.until(next.as_bytes()).await?)
            .map_err(|_| malformed("a field's value is not text"))?;
        fields.push((name, value));
        match reader.take(2).await?.as_slice() {
            b"\r\n" => {}
            b"--" => return Err(malformed("the form has no file")),
            _ => return Err(malformed("a boundary is malformed")),
        }
    }
}

/// The file as a stream of exactly `length` bytes; before the last of them is handed on, what follows must be
/// `closing` and then nothing — otherwise `broken` is set and the stream fails, so nothing is stored.
fn file_body(reader: Reader, length: u64, closing: Vec<u8>, broken: Arc<AtomicBool>) -> Body {
    let stream = futures_util::stream::try_unfold(
        (reader, length, closing, broken),
        |(mut reader, left, closing, broken)| async move {
            if left == 0 {
                return Ok(None);
            }
            if reader.buf.is_empty() && !reader.fill().await.unwrap_or(false) {
                broken.store(true, Ordering::Release);
                return Err(std::io::Error::other("the form ends before its file does"));
            }
            let usable = u64::try_from(reader.buf.len())
                .unwrap_or(u64::MAX)
                .min(left);
            let cut = usize::try_from(usable).unwrap_or(reader.buf.len());
            let remaining = left.saturating_sub(usable);
            if remaining == 0 {
                // The last bytes: what follows them must be the closing delimiter and the end of the body.
                while reader.buf.len() < cut.saturating_add(closing.len()) {
                    if !reader.fill().await.unwrap_or(false) {
                        break;
                    }
                }
                let ends = reader.buf.get(cut..) == Some(closing.as_slice())
                    && reader.stream.next().await.is_none();
                if !ends {
                    broken.store(true, Ordering::Release);
                    return Err(std::io::Error::other(
                        "the form does not end where its length says",
                    ));
                }
            }
            let chunk = Bytes::from(reader.buf.drain(..cut).collect::<Vec<u8>>());
            Ok(Some((chunk, (reader, remaining, closing, broken))))
        },
    );
    Body::from_stream(stream)
}
