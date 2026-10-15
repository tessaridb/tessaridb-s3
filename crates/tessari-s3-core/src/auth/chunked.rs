//! The aws-chunked body: a push parser that frames chunks, verifies each signed chunk through the chain as it
//! arrives, enforces the decoded length, and reads the trailer. It holds at most one chunk, and a chunk larger than
//! the ceiling is refused before it is buffered.

use super::chunk::{ChunkChain, parse_chunk_header};
use super::verified::{PayloadHash, Verified};
use super::{AuthError, AuthResult};

/// Largest chunk accepted (16 MiB): SDKs send 64 KiB to a few MiB, and the frame states its size before its bytes.
const MAX_CHUNK: u64 = 16 << 20;
/// Longest header or trailer line.
const MAX_LINE: usize = 4096;

/// A trailing checksum: the header name (lowercase) and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trailer {
    /// e.g. `x-amz-checksum-crc32`.
    pub name: String,
    /// The base64 value.
    pub value: String,
}

#[derive(Debug)]
enum State {
    Header,
    Data {
        size: u64,
        signature: Option<String>,
    },
    Trailers,
    End,
}

/// Decodes one aws-chunked body.
#[derive(Debug)]
pub struct ChunkedDecoder {
    chain: Option<ChunkChain>,
    trailer_expected: Option<String>,
    decoded_len: u64,
    received: u64,
    pending: Vec<u8>,
    state: State,
    trailer: Option<Trailer>,
    trailer_signature: Option<String>,
    /// The last trailer line ended in a bare `\n`; some clients then send `\r\n` before the blank line.
    bare_newline: bool,
}

impl ChunkedDecoder {
    /// A decoder for a request whose payload hash is a `STREAMING-…` marker, carrying `x-amz-decoded-content-length`
    /// and, for the trailer variants, the `x-amz-trailer` header naming the one trailer it will send.
    ///
    /// # Errors
    /// [`AuthError::MalformedChunk`] when the payload is not streaming or a trailer variant names no trailer.
    pub fn new(
        verified: Verified,
        decoded_len: u64,
        trailer_header: Option<&str>,
    ) -> AuthResult<Self> {
        let trailer_expected = trailer_header.map(|name| name.trim().to_ascii_lowercase());
        let (signed, needs_trailer) = match verified.payload {
            PayloadHash::StreamingSigned => (true, false),
            PayloadHash::StreamingSignedTrailer => (true, true),
            PayloadHash::StreamingUnsignedTrailer => (false, true),
            PayloadHash::Sha256(_) | PayloadHash::Unsigned => {
                return Err(AuthError::MalformedChunk {
                    reason: "the payload is not aws-chunked",
                });
            }
        };
        if needs_trailer != trailer_expected.is_some() {
            return Err(AuthError::MalformedChunk {
                reason: "x-amz-trailer does not match the payload",
            });
        }
        let chain = signed.then(|| ChunkChain::new(verified, decoded_len));
        Ok(Self {
            chain,
            trailer_expected,
            decoded_len,
            received: 0,
            pending: Vec::new(),
            state: State::Header,
            trailer: None,
            trailer_signature: None,
            bare_newline: false,
        })
    }

    /// Feeds the next bytes of the body, appending the decoded data to `out`.
    ///
    /// # Errors
    /// Any framing, signature or length refusal; nothing decoded after a refusal may be used.
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<u8>) -> AuthResult<()> {
        self.pending.extend_from_slice(bytes);
        loop {
            let progressed = match std::mem::replace(&mut self.state, State::End) {
                State::Header => self.header()?,
                State::Data { size, signature } => self.data(size, signature, out)?,
                State::Trailers => self.trailers()?,
                State::End => {
                    if !self.pending.is_empty() {
                        return Err(AuthError::MalformedChunk {
                            reason: "bytes after the end of the body",
                        });
                    }
                    false
                }
            };
            if !progressed {
                return Ok(());
            }
        }
    }

    /// Ends the body: the final chunk, the decoded length and the trailer must all be complete.
    ///
    /// # Errors
    /// [`AuthError::MalformedChunk`] or [`AuthError::DecodedLengthMismatch`].
    pub fn finish(self) -> AuthResult<Option<Trailer>> {
        if !matches!(self.state, State::End) || !self.pending.is_empty() {
            return Err(AuthError::MalformedChunk {
                reason: "the body ended before the final chunk",
            });
        }
        if self.received != self.decoded_len {
            return Err(AuthError::DecodedLengthMismatch);
        }
        if let Some(chain) = &self.chain {
            chain.finish()?;
        }
        Ok(self.trailer)
    }

    /// Takes one line when a whole one is buffered: its text without the ending, and whether it ended in a bare
    /// `\n` rather than `\r\n`.
    fn line(&mut self) -> AuthResult<Option<(String, bool)>> {
        let Some(end) = self.pending.iter().position(|b| *b == b'\n') else {
            if self.pending.len() > MAX_LINE {
                return Err(AuthError::MalformedChunk {
                    reason: "a line longer than 4 KiB",
                });
            }
            return Ok(None);
        };
        let line: Vec<u8> = self.pending.drain(..=end).collect();
        let bare = !line.ends_with(b"\r\n");
        let text = String::from_utf8(line).map_err(|_| AuthError::MalformedChunk {
            reason: "a line is not UTF-8",
        })?;
        Ok(Some((text.trim_end_matches(['\r', '\n']).to_owned(), bare)))
    }

    fn header(&mut self) -> AuthResult<bool> {
        let Some((line, _)) = self.line()? else {
            self.state = State::Header;
            return Ok(false);
        };
        let (size, signature) = if self.chain.is_some() {
            let (size, signature) = parse_chunk_header(&line)?;
            (size, Some(signature.to_owned()))
        } else {
            let hex = line.split(';').next().unwrap_or("");
            let size = u64::from_str_radix(hex, 16).map_err(|_| AuthError::MalformedChunk {
                reason: "chunk size",
            })?;
            (size, None)
        };
        if size > MAX_CHUNK {
            return Err(AuthError::MalformedChunk {
                reason: "a chunk larger than 16 MiB",
            });
        }
        self.state = State::Data { size, signature };
        Ok(true)
    }

    fn data(
        &mut self,
        size: u64,
        signature: Option<String>,
        out: &mut Vec<u8>,
    ) -> AuthResult<bool> {
        let length = usize::try_from(size).map_err(|_| AuthError::MalformedChunk {
            reason: "chunk size",
        })?;
        let final_chunk = length == 0;
        // The final chunk of a trailer variant is followed by the trailer, not by an empty line.
        let framing = if final_chunk && self.trailer_expected.is_some() {
            0
        } else {
            2
        };
        let needed = length
            .checked_add(framing)
            .ok_or(AuthError::MalformedChunk {
                reason: "chunk size",
            })?;
        if self.pending.len() < needed {
            self.state = State::Data { size, signature };
            return Ok(false);
        }
        let chunk: Vec<u8> = self.pending.drain(..length).collect();
        if framing == 2 {
            let ending: Vec<u8> = self.pending.drain(..2).collect();
            if ending != b"\r\n" {
                return Err(AuthError::MalformedChunk {
                    reason: "a chunk not followed by CRLF",
                });
            }
        }
        match (&mut self.chain, signature) {
            (Some(chain), Some(signature)) => chain.verify_chunk(&chunk, &signature)?,
            (Some(_), None) => {
                return Err(AuthError::MalformedChunk {
                    reason: "an unsigned chunk in a signed body",
                });
            }
            (None, _) => {}
        }
        self.received = self
            .received
            .checked_add(size)
            .ok_or(AuthError::DecodedLengthMismatch)?;
        if self.received > self.decoded_len {
            return Err(AuthError::DecodedLengthMismatch);
        }
        out.extend_from_slice(&chunk);
        self.state = match (final_chunk, self.trailer_expected.is_some()) {
            (true, true) => State::Trailers,
            (true, false) => State::End,
            (false, _) => State::Header,
        };
        Ok(true)
    }

    fn trailers(&mut self) -> AuthResult<bool> {
        let Some((line, bare)) = self.line()? else {
            self.state = State::Trailers;
            return Ok(false);
        };
        let after_bare = std::mem::replace(&mut self.bare_newline, bare);
        if line.is_empty() && after_bare && !bare {
            // The `\r\n` some clients send after a trailer value already ended by `\n`: part of that line.
            self.state = State::Trailers;
            return Ok(true);
        }
        if line.is_empty() {
            if self.trailer.is_none() {
                return Err(AuthError::MalformedChunk {
                    reason: "the declared trailer is missing",
                });
            }
            if self.chain.is_some() && self.trailer_signature.is_none() {
                return Err(AuthError::MalformedChunk {
                    reason: "the trailer signature is missing",
                });
            }
            self.state = State::End;
            return Ok(true);
        }
        let (name, value) = line.split_once(':').ok_or(AuthError::MalformedChunk {
            reason: "a trailer line without a colon",
        })?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        if name == "x-amz-trailer-signature" {
            let trailer = self.trailer.as_ref().ok_or(AuthError::MalformedChunk {
                reason: "a signature before its trailer",
            })?;
            let signed = format!("{}:{}", trailer.name, trailer.value);
            let chain = self.chain.as_mut().ok_or(AuthError::MalformedChunk {
                reason: "a signature on an unsigned trailer",
            })?;
            chain.verify_trailer(&signed, &value)?;
            self.trailer_signature = Some(value);
        } else if Some(&name) == self.trailer_expected.as_ref() && self.trailer.is_none() {
            self.trailer = Some(Trailer { name, value });
        } else {
            return Err(AuthError::MalformedChunk {
                reason: "a trailer other than the one x-amz-trailer named",
            });
        }
        self.state = State::Trailers;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "chunked_tests.rs"]
mod tests;
