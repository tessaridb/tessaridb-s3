//! The streaming seed and the aws-chunked signature chain.

use tessari_s3_core::auth::{AuthError, ChunkChain, PayloadHash, parse_chunk_header};

use crate::{NOW, REGION, authorization, check};

// ---- streaming PUT: seed and chunk chain ----

const SEED: &str = "4f232c4386841ef735655705268965c44a0e4690baa4adea153f7db9fa80a0a9";
const CHUNK_1: &str = "ad80c730a21e5b8d04586a2213dd63b9a0e99e0e2307b0ade35a65485a288648";
const CHUNK_2: &str = "0055627c9e194cb4542bae2aa5492e3c1575bbb81b612b7d234b86a503ef5497";
const CHUNK_FINAL: &str = "b6c6ea8a5354eaf15b3cb7646744f4275b71ea724fed81ceb9323e279d449df9";
const STREAMING_SIGNED_HEADERS: &str = "content-encoding;content-length;host;x-amz-content-sha256;x-amz-date;x-amz-decoded-content-length;x-amz-storage-class";

fn streaming_seed() -> tessari_s3_core::auth::Verified {
    let owned = vec![
        ("Host", "s3.amazonaws.com".to_owned()),
        ("x-amz-date", "20130524T000000Z".to_owned()),
        ("x-amz-storage-class", "REDUCED_REDUNDANCY".to_owned()),
        (
            "Authorization",
            authorization(STREAMING_SIGNED_HEADERS, SEED),
        ),
        (
            "x-amz-content-sha256",
            "STREAMING-AWS4-HMAC-SHA256-PAYLOAD".to_owned(),
        ),
        ("Content-Encoding", "aws-chunked".to_owned()),
        ("x-amz-decoded-content-length", "66560".to_owned()),
        ("Content-Length", "66824".to_owned()),
    ];
    check(
        "PUT",
        "/examplebucket/chunkObject.txt",
        "",
        &owned,
        REGION,
        NOW,
    )
    .expect("published seed verifies")
}

#[test]
fn streaming_seed_and_every_chunk_reproduce_the_published_signatures() {
    let seed = streaming_seed();
    assert_eq!(seed.payload(), PayloadHash::StreamingSigned);
    let mut chain = ChunkChain::new(seed, 66_560);
    chain
        .verify_chunk(&[b'a'; 65_536], CHUNK_1)
        .expect("chunk 1");
    chain
        .verify_chunk(&[b'a'; 1_024], CHUNK_2)
        .expect("chunk 2");
    chain.verify_chunk(&[], CHUNK_FINAL).expect("final chunk");
    assert_eq!(chain.finish(), Ok(()));
}

#[test]
fn a_chunk_with_one_byte_changed_is_refused() {
    let mut chain = ChunkChain::new(streaming_seed(), 66_560);
    let mut data = [b'a'; 65_536];
    data[100] = b'b';
    assert_eq!(
        chain.verify_chunk(&data, CHUNK_1),
        Err(AuthError::SignatureMismatch)
    );
}

#[test]
fn chunks_swapped_or_skipped_break_the_chain() {
    let mut chain = ChunkChain::new(streaming_seed(), 66_560);
    assert_eq!(
        chain.verify_chunk(&[b'a'; 1_024], CHUNK_2),
        Err(AuthError::SignatureMismatch)
    );
}

#[test]
fn a_stream_that_ends_before_the_final_chunk_is_refused() {
    let mut chain = ChunkChain::new(streaming_seed(), 66_560);
    chain
        .verify_chunk(&[b'a'; 65_536], CHUNK_1)
        .expect("chunk 1");
    chain
        .verify_chunk(&[b'a'; 1_024], CHUNK_2)
        .expect("chunk 2");
    assert!(matches!(
        chain.finish(),
        Err(AuthError::MalformedChunk { .. })
    ));
}

#[test]
fn a_decoded_length_that_does_not_match_the_chunks_is_refused() {
    let mut chain = ChunkChain::new(streaming_seed(), 66_561);
    chain
        .verify_chunk(&[b'a'; 65_536], CHUNK_1)
        .expect("chunk 1");
    chain
        .verify_chunk(&[b'a'; 1_024], CHUNK_2)
        .expect("chunk 2");
    assert_eq!(
        chain.verify_chunk(&[], CHUNK_FINAL),
        Err(AuthError::DecodedLengthMismatch)
    );
}

#[test]
fn chunk_header_lines_parse_and_malformed_ones_are_refused() {
    assert_eq!(
        parse_chunk_header(&format!("10000;chunk-signature={CHUNK_1}")),
        Ok((65_536, CHUNK_1))
    );
    assert_eq!(
        parse_chunk_header(&format!("0;chunk-signature={CHUNK_FINAL}")),
        Ok((0, CHUNK_FINAL))
    );
    for line in [
        "",
        "10000",
        "zz;chunk-signature=00",
        "10000;chunk-signature=",
        "10000;signature=ab",
    ] {
        assert!(
            matches!(
                parse_chunk_header(line),
                Err(AuthError::MalformedChunk { .. })
            ),
            "{line:?}"
        );
    }
}
