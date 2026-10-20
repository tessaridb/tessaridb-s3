use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{ImageFormat, Rgb, RgbImage};
use tessari_s3_constants::MEDIA_MAX_INPUT_BYTES;

use super::{Encoding, Mode, RenderError, Target, render};

/// Fitted into a square of `side`, at the format's own quality.
const fn fit(side: u32, encoding: Encoding) -> Target {
    Target {
        width: side,
        height: side,
        mode: Mode::Fit,
        encoding,
        quality: None,
    }
}

/// Made to `width` by `height` as `mode` says, written as PNG so pixels compare exactly.
const fn boxed(width: u32, height: u32, mode: Mode) -> Target {
    Target {
        width,
        height,
        mode,
        encoding: Encoding::Png,
        quality: None,
    }
}

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
    let made = render(
        &png(2000, 1000),
        &[fit(320, Encoding::Jpeg), fit(1600, Encoding::Jpeg)],
    )
    .expect("rendered");
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
    let made = render(&png(100, 50), &[fit(320, Encoding::Jpeg)]).expect("rendered");
    assert_eq!((made[0].width, made[0].height), (100, 50));
}

#[test]
fn a_declared_size_past_the_pixel_bound_is_refused_before_decoding() {
    assert_eq!(
        render(&declared_png(100_000, 100_000), &[fit(320, Encoding::Jpeg)]),
        Err(RenderError::TooManyPixels)
    );
}

#[test]
fn an_original_past_the_byte_bound_and_one_that_is_no_image_are_refused() {
    let past = vec![0_u8; usize::try_from(MEDIA_MAX_INPUT_BYTES + 1).expect("fits")];
    assert_eq!(
        render(&past, &[fit(320, Encoding::Jpeg)]),
        Err(RenderError::TooLarge)
    );
    assert_eq!(
        render(b"not an image", &[fit(320, Encoding::Jpeg)]),
        Err(RenderError::Unreadable)
    );
}

/// The width and height an AVIF file declares in its `ispe` box: version and flags, then two big-endian words.
fn avif_size(bytes: &[u8]) -> (u32, u32) {
    let at = bytes
        .windows(4)
        .position(|window| window == b"ispe")
        .expect("an ispe box");
    let mut words = bytes[at..]
        .as_chunks::<4>()
        .0
        .iter()
        .skip(2)
        .map(|word| u32::from_be_bytes(*word));
    (
        words.next().expect("a width"),
        words.next().expect("a height"),
    )
}

#[test]
fn each_encoding_is_written_as_itself_at_the_fitted_size() {
    let made = render(
        &png(400, 200),
        &[
            fit(320, Encoding::Jpeg),
            fit(320, Encoding::Png),
            fit(320, Encoding::Webp),
            fit(320, Encoding::Avif),
        ],
    )
    .expect("rendered");
    let formats: Vec<ImageFormat> = made
        .iter()
        .map(|one| image::guess_format(&one.bytes).expect("a format"))
        .collect();
    assert_eq!(
        formats,
        vec![
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::WebP,
            ImageFormat::Avif
        ]
    );
    for one in &made[..3] {
        let back = image::load_from_memory(&one.bytes).expect("decodes");
        assert_eq!((back.width(), back.height()), (320, 160));
    }
    assert_eq!(avif_size(&made[3].bytes), (320, 160));
    assert_eq!((made[3].width, made[3].height), (320, 160));
}

fn sizes(input: &[u8], targets: &[Target]) -> Vec<(u32, u32)> {
    render(input, targets)
        .expect("rendered")
        .iter()
        .map(|one| (one.width, one.height))
        .collect()
}

#[test]
fn a_box_is_fitted_into_keeping_the_shape_and_never_enlarging() {
    assert_eq!(
        sizes(
            &png(2000, 1000),
            &[
                boxed(400, 400, Mode::Fit),
                boxed(1000, 100, Mode::Fit),
                boxed(4000, 4000, Mode::Fit),
            ]
        ),
        vec![(400, 200), (200, 100), (2000, 1000)]
    );
}

#[test]
fn cover_fills_the_box_from_the_middle_and_never_enlarges() {
    assert_eq!(
        sizes(
            &png(2000, 1000),
            &[boxed(400, 400, Mode::Cover), boxed(300, 100, Mode::Cover)]
        ),
        vec![(400, 400), (300, 100)]
    );
    // Three bands, red, green and blue: a square cover keeps the green middle and crops both sides.
    let bands = RgbImage::from_fn(300, 100, |x, _| match x {
        0..100 => Rgb([255, 0, 0]),
        100..200 => Rgb([0, 255, 0]),
        _ => Rgb([0, 0, 255]),
    });
    let mut original = Cursor::new(Vec::new());
    bands
        .write_to(&mut original, ImageFormat::Png)
        .expect("png written");
    let made = render(&original.into_inner(), &[boxed(400, 400, Mode::Cover)]).expect("rendered");
    assert_eq!(
        (made[0].width, made[0].height),
        (100, 100),
        "never enlarged"
    );
    let back = image::load_from_memory(&made[0].bytes)
        .expect("decodes")
        .to_rgb8();
    for (x, y) in [(0, 0), (99, 0), (0, 99), (99, 99), (50, 50)] {
        assert_eq!(back.get_pixel(x, y), &Rgb([0, 255, 0]), "pixel {x},{y}");
    }
}

#[test]
fn exact_makes_the_box_whatever_the_shape() {
    assert_eq!(
        sizes(
            &png(400, 200),
            &[boxed(100, 300, Mode::Exact), boxed(800, 800, Mode::Exact)]
        ),
        vec![(100, 300), (800, 800)]
    );
}

#[test]
fn a_named_quality_is_honoured_on_each_lossy_format() {
    for encoding in [Encoding::Jpeg, Encoding::Webp, Encoding::Avif] {
        let at = |quality| Target {
            quality: Some(quality),
            ..fit(400, encoding)
        };
        let made = render(&png(400, 400), &[at(10), at(95)]).expect("rendered");
        assert!(
            made[0].bytes.len() < made[1].bytes.len(),
            "{encoding:?}: quality 10 is {} bytes, 95 is {}",
            made[0].bytes.len(),
            made[1].bytes.len()
        );
    }
}

/// A JPEG of `width` × `height` carrying EXIF with `orientation` and a GPS latitude reference.
fn photo(width: u32, height: u32, orientation: u16) -> Vec<u8> {
    let image = RgbImage::from_fn(width, height, |x, _| {
        Rgb([u8::try_from(x % 256).unwrap_or(0), 80, 160])
    });
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 90)
        .encode_image(&image)
        .expect("jpeg written");
    // Little-endian TIFF: IFD0 at 8 with Orientation and the GPS IFD pointer, the GPS IFD at 38 with one entry.
    let mut tiff = b"II*\0".to_vec();
    tiff.extend_from_slice(&8_u32.to_le_bytes());
    tiff.extend_from_slice(&2_u16.to_le_bytes());
    tiff.extend_from_slice(&0x0112_u16.to_le_bytes());
    tiff.extend_from_slice(&3_u16.to_le_bytes());
    tiff.extend_from_slice(&1_u32.to_le_bytes());
    tiff.extend_from_slice(&u32::from(orientation).to_le_bytes());
    tiff.extend_from_slice(&0x8825_u16.to_le_bytes());
    tiff.extend_from_slice(&4_u16.to_le_bytes());
    tiff.extend_from_slice(&1_u32.to_le_bytes());
    tiff.extend_from_slice(&38_u32.to_le_bytes());
    tiff.extend_from_slice(&0_u32.to_le_bytes());
    tiff.extend_from_slice(&1_u16.to_le_bytes());
    tiff.extend_from_slice(&0x0001_u16.to_le_bytes());
    tiff.extend_from_slice(&2_u16.to_le_bytes());
    tiff.extend_from_slice(&2_u32.to_le_bytes());
    tiff.extend_from_slice(b"N\0\0\0");
    tiff.extend_from_slice(&0_u32.to_le_bytes());
    let mut segment = b"Exif\0\0".to_vec();
    segment.extend_from_slice(&tiff);
    let length = segment
        .len()
        .checked_add(2)
        .and_then(|length| u16::try_from(length).ok())
        .expect("a short segment");
    let mut bytes = encoded[..2].to_vec();
    bytes.extend_from_slice(&[0xFF, 0xE1]);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(&segment);
    bytes.extend_from_slice(&encoded[2..]);
    bytes
}

#[test]
fn a_photo_is_turned_upright_before_it_is_sized_and_keeps_no_metadata() {
    let original = photo(300, 100, 6);
    assert!(original.windows(4).any(|window| window == b"Exif"));
    let made = render(
        &original,
        &[
            fit(3000, Encoding::Jpeg),
            boxed(50, 100, Mode::Exact),
            fit(150, Encoding::Webp),
            fit(150, Encoding::Avif),
        ],
    )
    .expect("rendered");
    assert_eq!(
        (made[0].width, made[0].height),
        (100, 300),
        "rotated a quarter turn"
    );
    assert_eq!((made[2].width, made[2].height), (50, 150));
    for one in &made {
        assert!(
            !one.bytes.windows(4).any(|window| window == b"Exif"),
            "no EXIF, so no GPS, in a rendition"
        );
    }
    assert_eq!(
        sizes(&photo(300, 100, 1), &[fit(3000, Encoding::Jpeg)]),
        vec![(300, 100)],
        "an upright photo stays as it is"
    );
}
