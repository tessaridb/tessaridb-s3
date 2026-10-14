use std::io::Cursor;

use image::{ImageFormat, Rgb, RgbImage};
use tessari_s3_constants::MEDIA_MAX_INPUT_BYTES;

use super::{RenderError, render_jpegs};

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = RgbImage::from_fn(width, height, |x, y| {
        Rgb([
            u8::try_from(x % 256).unwrap_or(0),
            u8::try_from(y % 256).unwrap_or(0),
            90,
        ])
    });
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, ImageFormat::Png)
        .expect("png written");
    out.into_inner()
}

/// One PNG chunk: length, kind, data and its CRC.
fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut covered = kind.to_vec();
    covered.extend_from_slice(data);
    let crc = crc_fast::checksum(crc_fast::CrcAlgorithm::Crc32IsoHdlc, &covered);
    let mut bytes = u32::try_from(data.len())
        .expect("a short chunk")
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(&covered);
    bytes.extend_from_slice(&u32::try_from(crc).expect("a 32-bit crc").to_be_bytes());
    bytes
}

/// A small PNG file declaring `width` × `height`: well-formed chunks, a few bytes of compressed data — the shape of a
/// decompression bomb, which only its declared size gives away.
fn declared_png(width: u32, height: u32) -> Vec<u8> {
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(chunk(b"IHDR", &header));
    bytes.extend(chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01],
    ));
    bytes.extend(chunk(b"IEND", &[]));
    bytes
}

#[test]
fn an_original_is_fitted_into_each_box_as_jpeg_keeping_its_shape() {
    let made = render_jpegs(&png(2000, 1000), &[320, 1600]).expect("rendered");
    let sizes: Vec<(u32, u32)> = made.iter().map(|one| (one.width, one.height)).collect();
    assert_eq!(sizes, vec![(320, 160), (1600, 800)]);
    for one in &made {
        assert_eq!(
            image::guess_format(&one.bytes).expect("a format"),
            ImageFormat::Jpeg
        );
        let back = image::load_from_memory(&one.bytes).expect("decodes");
        assert_eq!((back.width(), back.height()), (one.width, one.height));
    }
}

#[test]
fn a_small_original_is_never_enlarged() {
    let made = render_jpegs(&png(100, 50), &[320]).expect("rendered");
    assert_eq!((made[0].width, made[0].height), (100, 50));
}

#[test]
fn a_declared_size_past_the_pixel_bound_is_refused_before_decoding() {
    assert_eq!(
        render_jpegs(&declared_png(100_000, 100_000), &[320]),
        Err(RenderError::TooManyPixels)
    );
}

#[test]
fn an_original_past_the_byte_bound_and_one_that_is_no_image_are_refused() {
    let past = vec![0_u8; usize::try_from(MEDIA_MAX_INPUT_BYTES + 1).expect("fits")];
    assert_eq!(render_jpegs(&past, &[320]), Err(RenderError::TooLarge));
    assert_eq!(
        render_jpegs(b"not an image", &[320]),
        Err(RenderError::Unreadable)
    );
}
