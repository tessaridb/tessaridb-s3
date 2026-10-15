//! Image renditions: an original decoded once, under a size and a pixel bound checked before decoding, and written in
//! each requested encoding fitted into each requested box — never enlarged. Pure and synchronous: a caller on the async
//! runtime runs it through `spawn_blocking`.

use std::borrow::Cow;
use std::io::Cursor;

use image::codecs::avif::AvifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageEncoder, ImageReader, Limits};
use tessari_s3_constants::{
    MEDIA_AVIF_QUALITY, MEDIA_AVIF_SPEED, MEDIA_JPEG_QUALITY, MEDIA_MAX_INPUT_BYTES,
    MEDIA_MAX_PIXELS, MEDIA_WEBP_QUALITY,
};

/// The file format a rendition is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// JPEG, lossy, no transparency.
    Jpeg,
    /// PNG, lossless.
    Png,
    /// WebP, lossy.
    Webp,
    /// AVIF, lossy.
    Avif,
}

/// One rendition: the file and the size it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendition {
    /// The encoded file.
    pub bytes: Vec<u8>,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
}

/// Why an original gets no renditions.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    /// The original is larger than a worker reads.
    #[error("the original is larger than a media worker reads")]
    TooLarge,
    /// The original declares more pixels than a worker decodes.
    #[error("the original declares more pixels than a media worker decodes")]
    TooManyPixels,
    /// The original is not an image this server reads.
    #[error("the original is not an image this server reads")]
    Unreadable,
    /// A rendition could not be written.
    #[error("a rendition could not be encoded")]
    Encode,
}

/// The original `input` fitted into the square box of each output's side in pixels and written in its encoding, in
/// that order.
///
/// # Errors
/// [`RenderError`] when the original is too large, declares too many pixels, cannot be read, or a rendition cannot be
/// encoded.
pub fn render(input: &[u8], outputs: &[(u32, Encoding)]) -> Result<Vec<Rendition>, RenderError> {
    if u64::try_from(input.len()).map_or(true, |length| length > MEDIA_MAX_INPUT_BYTES) {
        return Err(RenderError::TooLarge);
    }
    let reader = || {
        ImageReader::new(Cursor::new(input))
            .with_guessed_format()
            .map_err(|_| RenderError::Unreadable)
    };
    // The header alone is read first: the declared size decides whether decoding may start at all.
    let (width, height) = reader()?
        .into_dimensions()
        .map_err(|_| RenderError::Unreadable)?;
    if u64::from(width).saturating_mul(u64::from(height)) > MEDIA_MAX_PIXELS {
        return Err(RenderError::TooManyPixels);
    }
    let mut decoder = reader()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    decoder.limits(limits);
    let original = decoder.decode().map_err(|_| RenderError::Unreadable)?;
    outputs
        .iter()
        .map(|(side, encoding)| {
            // Borrowed when it already fits: an original is up to the pixel bound, and a copy per output is not free.
            let fitted = if original.width() <= *side && original.height() <= *side {
                Cow::Borrowed(&original)
            } else {
                Cow::Owned(original.resize(*side, *side, FilterType::Lanczos3))
            };
            Ok(Rendition {
                bytes: encode(&fitted, *encoding)?,
                width: fitted.width(),
                height: fitted.height(),
            })
        })
        .collect()
}

/// `image` written in `encoding`; JPEG drops transparency, the others keep it.
fn encode(image: &DynamicImage, encoding: Encoding) -> Result<Vec<u8>, RenderError> {
    let mut bytes = Vec::new();
    match encoding {
        Encoding::Jpeg => JpegEncoder::new_with_quality(&mut bytes, MEDIA_JPEG_QUALITY)
            .encode_image(&image.to_rgb8())
            .map_err(|_| RenderError::Encode)?,
        Encoding::Png => {
            let rgba = image.to_rgba8();
            PngEncoder::new(&mut bytes)
                .write_image(&rgba, rgba.width(), rgba.height(), ExtendedColorType::Rgba8)
                .map_err(|_| RenderError::Encode)?;
        }
        Encoding::Webp => {
            let rgba = image.to_rgba8();
            let written = webp::Encoder::from_rgba(&rgba, rgba.width(), rgba.height())
                .encode_simple(false, f32::from(MEDIA_WEBP_QUALITY))
                .map_err(|_| RenderError::Encode)?;
            bytes.extend_from_slice(&written);
        }
        Encoding::Avif => {
            let rgba = image.to_rgba8();
            AvifEncoder::new_with_speed_quality(&mut bytes, MEDIA_AVIF_SPEED, MEDIA_AVIF_QUALITY)
                .write_image(&rgba, rgba.width(), rgba.height(), ExtendedColorType::Rgba8)
                .map_err(|_| RenderError::Encode)?;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "media_image_tests.rs"]
mod tests;
