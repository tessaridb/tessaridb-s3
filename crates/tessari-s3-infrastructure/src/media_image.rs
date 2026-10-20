//! Image renditions: an original decoded once, under a size and a pixel bound checked before decoding, turned upright by
//! its EXIF orientation, and written in each requested encoding and quality, made to each requested box: fitted into it
//! or covering it (never enlarged), or exactly it. Pure and synchronous: a caller on the async runtime runs it through
//! `spawn_blocking`. Nothing of the original's metadata is written: each rendition is encoded from pixels.

use std::borrow::Cow;
use std::io::Cursor;

use image::codecs::avif::AvifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageReader, Limits};
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

/// How an original is made to a target's box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Inside the box, its shape kept, never enlarged.
    Fit,
    /// Filling the box, the middle kept and the rest cropped, never enlarged.
    Cover,
    /// Exactly the box, its shape not kept.
    Exact,
}

/// One rendition to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The box's width in pixels.
    pub width: u32,
    /// The box's height in pixels.
    pub height: u32,
    /// How the original is made to it.
    pub mode: Mode,
    /// The file format.
    pub encoding: Encoding,
    /// The quality, 1..=100, of a lossy encoding; `None` for its own default. PNG ignores it.
    pub quality: Option<u8>,
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

/// The original `input` made to each target's box and written in its encoding, in that order.
///
/// # Errors
/// [`RenderError`] when the original is too large, declares too many pixels, cannot be read, or a rendition cannot be
/// encoded.
pub fn render(input: &[u8], targets: &[Target]) -> Result<Vec<Rendition>, RenderError> {
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
    let mut reader = reader()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| RenderError::Unreadable)?;
    // A camera stores the sensor's rows and says how to turn them; a rendition is sized upright.
    let orientation = decoder.orientation().map_err(|_| RenderError::Unreadable)?;
    let mut original = DynamicImage::from_decoder(decoder).map_err(|_| RenderError::Unreadable)?;
    original.apply_orientation(orientation);
    targets
        .iter()
        .map(|target| {
            let made = shaped(&original, target);
            Ok(Rendition {
                bytes: encode(&made, target.encoding, target.quality)?,
                width: made.width(),
                height: made.height(),
            })
        })
        .collect()
}

/// `original` made to `target`'s box; borrowed when it already is what the box asks, since an original is up to the
/// pixel bound and a copy per output is not free.
fn shaped<'a>(original: &'a DynamicImage, target: &Target) -> Cow<'a, DynamicImage> {
    let (width, height) = (target.width, target.height);
    match target.mode {
        Mode::Fit if original.width() <= width && original.height() <= height => {
            Cow::Borrowed(original)
        }
        Mode::Fit => Cow::Owned(original.resize(width, height, FilterType::Lanczos3)),
        Mode::Exact if original.width() == width && original.height() == height => {
            Cow::Borrowed(original)
        }
        Mode::Exact => Cow::Owned(original.resize_exact(width, height, FilterType::Lanczos3)),
        Mode::Cover => {
            let (crop_width, crop_height) =
                cover_crop(original.width(), original.height(), width, height);
            let cropped = original.crop_imm(
                original.width().saturating_sub(crop_width) / 2,
                original.height().saturating_sub(crop_height) / 2,
                crop_width,
                crop_height,
            );
            // Smaller than the box, the crop is kept as it is: covering never enlarges.
            if crop_width > width {
                Cow::Owned(cropped.resize_exact(width, height, FilterType::Lanczos3))
            } else {
                Cow::Owned(cropped)
            }
        }
    }
}

/// The largest region of `width`:`height` shape inside an original of `original_width` × `original_height`.
fn cover_crop(original_width: u32, original_height: u32, width: u32, height: u32) -> (u32, u32) {
    let (wide, tall) = (u64::from(original_width), u64::from(original_height));
    let (box_wide, box_tall) = (u64::from(width), u64::from(height));
    // Each side is at most its original's, so it fits back into a `u32`.
    let shrink = |side: u64, cap: u32| u32::try_from(side).map_or(cap, |side| side.clamp(1, cap));
    if wide.saturating_mul(box_tall) <= tall.saturating_mul(box_wide) {
        let crop_tall = wide
            .saturating_mul(box_tall)
            .checked_div(box_wide)
            .unwrap_or(0);
        (original_width, shrink(crop_tall, original_height))
    } else {
        let crop_wide = tall
            .saturating_mul(box_wide)
            .checked_div(box_tall)
            .unwrap_or(0);
        (shrink(crop_wide, original_width), original_height)
    }
}

/// `image` written in `encoding` at `quality`, or the encoding's own default; JPEG drops transparency, the others keep
/// it.
fn encode(
    image: &DynamicImage,
    encoding: Encoding,
    quality: Option<u8>,
) -> Result<Vec<u8>, RenderError> {
    let mut bytes = Vec::new();
    match encoding {
        Encoding::Jpeg => {
            JpegEncoder::new_with_quality(&mut bytes, quality.unwrap_or(MEDIA_JPEG_QUALITY))
                .encode_image(&image.to_rgb8())
                .map_err(|_| RenderError::Encode)?
        }
        Encoding::Png => {
            let rgba = image.to_rgba8();
            PngEncoder::new(&mut bytes)
                .write_image(&rgba, rgba.width(), rgba.height(), ExtendedColorType::Rgba8)
                .map_err(|_| RenderError::Encode)?;
        }
        Encoding::Webp => {
            let rgba = image.to_rgba8();
            let written = webp::Encoder::from_rgba(&rgba, rgba.width(), rgba.height())
                .encode_simple(false, f32::from(quality.unwrap_or(MEDIA_WEBP_QUALITY)))
                .map_err(|_| RenderError::Encode)?;
            bytes.extend_from_slice(&written);
        }
        Encoding::Avif => {
            let rgba = image.to_rgba8();
            AvifEncoder::new_with_speed_quality(
                &mut bytes,
                MEDIA_AVIF_SPEED,
                quality.unwrap_or(MEDIA_AVIF_QUALITY),
            )
            .write_image(&rgba, rgba.width(), rgba.height(), ExtendedColorType::Rgba8)
            .map_err(|_| RenderError::Encode)?;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "media_image_tests.rs"]
mod tests;
