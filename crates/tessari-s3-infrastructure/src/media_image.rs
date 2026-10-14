//! Image renditions: an original decoded once, under a size and a pixel bound checked before decoding, and written as
//! JPEG fitted into each requested box — never enlarged. Pure and synchronous: a caller on the async runtime runs it
//! through `spawn_blocking`.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ImageReader, Limits};
use tessari_s3_constants::{MEDIA_JPEG_QUALITY, MEDIA_MAX_INPUT_BYTES, MEDIA_MAX_PIXELS};

/// One rendition: JPEG bytes and the size they show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendition {
    /// The JPEG file.
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

/// The original `input` fitted into each square box of `boxes` pixels, in that order.
///
/// # Errors
/// [`RenderError`] when the original is too large, declares too many pixels, cannot be read, or a rendition cannot be
/// encoded.
pub fn render_jpegs(input: &[u8], boxes: &[u32]) -> Result<Vec<Rendition>, RenderError> {
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
    boxes
        .iter()
        .map(|side| {
            let fitted = if original.width() <= *side && original.height() <= *side {
                original.to_rgb8()
            } else {
                original
                    .resize(*side, *side, FilterType::Lanczos3)
                    .to_rgb8()
            };
            let mut bytes = Vec::new();
            JpegEncoder::new_with_quality(&mut bytes, MEDIA_JPEG_QUALITY)
                .encode_image(&fitted)
                .map_err(|_| RenderError::Encode)?;
            Ok(Rendition {
                bytes,
                width: fitted.width(),
                height: fitted.height(),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "media_image_tests.rs"]
mod tests;
