use super::{ChunkedDecoder, Trailer};
use crate::auth::signing::{CredentialScope, SigningKey};
use crate::auth::time::AmzDateTime;
use crate::auth::verified::{PayloadHash, Verified};
use crate::auth::{AuthError, AuthorizationHeader, SignedRequest, verify_header};
use tessari_s3_types::SecretKey;

const SEED: &str = "4f232c4386841ef735655705268965c44a0e4690baa4adea153f7db9fa80a0a9";
const CHUNK_1: &str = "ad80c730a21e5b8d04586a2213dd63b9a0e99e0e2307b0ade35a65485a288648";
const CHUNK_2: &str = "0055627c9e194cb4542bae2aa5492e3c1575bbb81b612b7d234b86a503ef5497";
const CHUNK_FINAL: &str = "b6c6ea8a5354eaf15b3cb7646744f4275b71ea724fed81ceb9323e279d449df9";

/// AWS's published streaming PUT, authenticated, ready to decode its body.
fn published_seed() -> Verified {
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=\
         content-encoding;content-length;host;x-amz-content-sha256;x-amz-date;x-amz-decoded-content-length;\
         x-amz-storage-class,Signature={SEED}"
    );
    let headers = [
        ("Host", "s3.amazonaws.com"),
        ("x-amz-date", "20130524T000000Z"),
        ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
        ("Authorization", authorization.as_str()),
        ("x-amz-content-sha256", "STREAMING-AWS4-HMAC-SHA256-PAYLOAD"),
        ("Content-Encoding", "aws-chunked"),
        ("x-amz-decoded-content-length", "66560"),
        ("Content-Length", "66824"),
    ];
    let request = SignedRequest {
        method: "PUT",
        raw_path: "/examplebucket/chunkObject.txt",
        raw_query: "",
        headers: &headers,
    };
    let parsed = AuthorizationHeader::parse(&authorization).expect("header");
    let secret = SecretKey::new("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned());
    verify_header(&request, &parsed, &secret, "us-east-1", 1_369_353_600)
        .expect("published seed verifies")
}

fn published_body() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("10000;chunk-signature={CHUNK_1}\r\n").as_bytes());
    body.extend_from_slice(&[b'a'; 65_536]);
    body.extend_from_slice(format!("\r\n400;chunk-signature={CHUNK_2}\r\n").as_bytes());
    body.extend_from_slice(&[b'a'; 1_024]);
    body.extend_from_slice(format!("\r\n0;chunk-signature={CHUNK_FINAL}\r\n\r\n").as_bytes());
    body
}

fn decode(decoder: &mut ChunkedDecoder, body: &[u8], slice: usize) -> Result<Vec<u8>, AuthError> {
    let mut out = Vec::new();
    for piece in body.chunks(slice) {
        decoder.push(piece, &mut out)?;
    }
    Ok(out)
}

#[test]
fn the_published_streaming_body_decodes_whatever_the_slicing() {
    let body = published_body();
    assert_eq!(
        body.len(),
        66_824,
        "the framed length is the published Content-Length"
    );
    for slice in [1, 7, 8_192, 100_000] {
        let mut decoder = ChunkedDecoder::new(published_seed(), 66_560, None).expect("decoder");
        let data = decode(&mut decoder, &body, slice).expect("decodes");
        assert_eq!(data.len(), 66_560, "slice {slice}");
        assert!(data.iter().all(|b| *b == b'a'));
        assert_eq!(decoder.finish(), Ok(None), "slice {slice}");
    }
}

#[test]
fn one_changed_data_byte_breaks_the_chain() {
    let mut body = published_body();
    let first_data = body.iter().position(|b| *b == b'a').expect("data") + 100;
    body[first_data] = b'b';
    let mut decoder = ChunkedDecoder::new(published_seed(), 66_560, None).expect("decoder");
    assert_eq!(
        decode(&mut decoder, &body, 4_096),
        Err(AuthError::SignatureMismatch)
    );
}

#[test]
fn a_truncated_body_is_refused_at_the_end() {
    let body = published_body();
    let mut decoder = ChunkedDecoder::new(published_seed(), 66_560, None).expect("decoder");
    decode(&mut decoder, &body[..body.len() - 40], 4_096).expect("a prefix decodes");
    assert!(matches!(
        decoder.finish(),
        Err(AuthError::MalformedChunk { .. })
    ));
}

#[test]
fn a_chunk_declared_larger_than_the_ceiling_is_refused_before_it_is_buffered() {
    let body = format!("1000001;chunk-signature={CHUNK_1}\r\n");
    let mut decoder = ChunkedDecoder::new(published_seed(), 66_560, None).expect("decoder");
    let refusal = decode(&mut decoder, body.as_bytes(), 1_000);
    assert_eq!(
        refusal,
        Err(AuthError::MalformedChunk {
            reason: "a chunk larger than 16 MiB"
        })
    );
}

/// A request already authenticated with an unsigned-trailer payload.
fn unsigned_trailer() -> Verified {
    let scope =
        CredentialScope::parse("20130524", "us-east-1", "s3", "aws4_request").expect("scope");
    let signing_key =
        SigningKey::derive(&SecretKey::new("secret".to_owned()), &scope).expect("key");
    Verified {
        access_key: "AK".to_owned(),
        scope,
        datetime: AmzDateTime::parse("20130524T000000Z").expect("date"),
        signing_key,
        signature: [7; 32],
        payload: PayloadHash::StreamingUnsignedTrailer,
    }
}

#[test]
fn an_unsigned_body_with_a_trailing_checksum_decodes_either_line_ending() {
    for ending in ["\r\n", "\n\r\n"] {
        let body = format!("5\r\nhello\r\n0\r\nx-amz-checksum-crc32:NhCmhg=={ending}\r\n");
        let mut decoder = ChunkedDecoder::new(unsigned_trailer(), 5, Some("x-amz-checksum-crc32"))
            .expect("decoder");
        assert_eq!(
            decode(&mut decoder, body.as_bytes(), 3).as_deref(),
            Ok(&b"hello"[..])
        );
        let trailer = Trailer {
            name: "x-amz-checksum-crc32".to_owned(),
            value: "NhCmhg==".to_owned(),
        };
        assert_eq!(decoder.finish(), Ok(Some(trailer)), "{ending:?}");
    }
}

#[test]
fn a_trailer_other_than_the_declared_one_or_a_wrong_length_is_refused() {
    let body = "5\r\nhello\r\n0\r\nx-amz-checksum-sha256:abc=\r\n\r\n";
    let mut decoder =
        ChunkedDecoder::new(unsigned_trailer(), 5, Some("x-amz-checksum-crc32")).expect("decoder");
    let refusal = decode(&mut decoder, body.as_bytes(), 64);
    assert_eq!(
        refusal,
        Err(AuthError::MalformedChunk {
            reason: "a trailer other than the one x-amz-trailer named"
        })
    );
    let body = "5\r\nhello\r\n0\r\nx-amz-checksum-crc32:NhCmhg==\r\n\r\n";
    let mut decoder =
        ChunkedDecoder::new(unsigned_trailer(), 6, Some("x-amz-checksum-crc32")).expect("decoder");
    decode(&mut decoder, body.as_bytes(), 64).expect("the frames are well formed");
    assert_eq!(
        decoder.finish(),
        Err(AuthError::DecodedLengthMismatch),
        "known only once the body has ended"
    );
}
