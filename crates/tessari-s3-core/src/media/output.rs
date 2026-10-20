//! A rendition's spelling: `kind/size/format`, a size being a side or a box with a mode, an optional `@quality` on a
//! lossy still format; and the file it is written to.

use tessari_s3_constants::MEDIA_MAX_PIXELS;

use super::{ImageFormat, MAX_SIDE, MIN_SIDE, VIDEO_SIZES, VideoFormat};

/// How an image is made to a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// Inside the box, its shape kept, never enlarged.
    Fit,
    /// Filling the box, the middle kept and the rest cropped, never enlarged.
    Cover,
    /// Exactly the box, its shape not kept.
    Exact,
}

impl Fit {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "fit" => Some(Self::Fit),
            "cover" => Some(Self::Cover),
            "exact" => Some(Self::Exact),
            _ => None,
        }
    }

    /// Its spelling after a box: none for `fit`, the default.
    const fn suffix(self) -> &'static str {
        match self {
            Self::Fit => "",
            Self::Cover => "-cover",
            Self::Exact => "-exact",
        }
    }
}

/// The size an image or a poster is made at, each side within [`MIN_SIDE`] and [`MAX_SIDE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Size {
    /// Fitted into a square of this side.
    Side(u32),
    /// Made to a box of `width` by `height` as `mode` says.
    Box {
        /// The box's width in pixels.
        width: u32,
        /// The box's height in pixels.
        height: u32,
        /// How the image is made to it.
        mode: Fit,
    },
}

impl Size {
    /// `<side>`, or `<W>x<H>` with an optional `-fit`, `-cover` or `-exact`.
    fn parse(name: &str) -> Result<Self, OutputError> {
        let (dimensions, mode) = match name.split_once('-') {
            Some((dimensions, mode)) => (
                dimensions,
                Some(Fit::parse(mode).ok_or(OutputError::Unknown)?),
            ),
            None => (name, None),
        };
        match dimensions.split_once('x') {
            Some((width, height)) => {
                let (width, height) = (side(width)?, side(height)?);
                // A box is made in memory whole, and `exact` may enlarge: it holds no more than an original may.
                if u64::from(width).saturating_mul(u64::from(height)) > MEDIA_MAX_PIXELS {
                    return Err(OutputError::BoxTooLarge);
                }
                Ok(Self::Box {
                    width,
                    height,
                    mode: mode.unwrap_or(Fit::Fit),
                })
            }
            None if mode.is_some() => {
                side(dimensions)?;
                Err(OutputError::ModeNeedsBox)
            }
            None => Ok(Self::Side(side(dimensions)?)),
        }
    }

    /// Its spelling, which is also how its file is named.
    fn name(self) -> String {
        match self {
            Self::Side(side) => side.to_string(),
            Self::Box {
                width,
                height,
                mode,
            } => format!("{width}x{height}{}", mode.suffix()),
        }
    }
}

/// A side in pixels: digits only, within [`MIN_SIDE`] and [`MAX_SIDE`].
fn side(digits: &str) -> Result<u32, OutputError> {
    number(digits)?
        .filter(|side| (MIN_SIDE..=MAX_SIDE).contains(side))
        .ok_or(OutputError::SizeOutOfRange)
}

/// A number spelled with digits only (`None` when too large for a `u32`); anything else is not one.
fn number(digits: &str) -> Result<Option<u32>, OutputError> {
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(OutputError::Unknown);
    }
    Ok(digits.parse().ok())
}

/// `<format>` or `<format>@<quality>`; a quality only on a lossy format and within 1..=100.
fn still(name: &str) -> Result<(ImageFormat, Option<u8>), OutputError> {
    let (format, quality) = match name.split_once('@') {
        Some((format, quality)) => (format, Some(quality)),
        None => (name, None),
    };
    let format = ImageFormat::parse(format).ok_or(OutputError::Unknown)?;
    let Some(quality) = quality else {
        return Ok((format, None));
    };
    let quality = number(quality)?
        .filter(|quality| (1..=100).contains(quality))
        .and_then(|quality| u8::try_from(quality).ok())
        .ok_or(OutputError::QualityOutOfRange)?;
    if format == ImageFormat::Png {
        return Err(OutputError::QualityOnLossless);
    }
    Ok((format, Some(quality)))
}

/// `@<quality>` when one is named.
fn quality_suffix(quality: Option<u8>, mark: &str) -> String {
    quality.map_or_else(String::new, |quality| format!("{mark}{quality}"))
}

/// Why a spelling is not an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutputError {
    /// Not `kind/size/format`, or a part not on its list.
    #[error("an output is image, video or poster, a size and a listed format")]
    Unknown,
    /// A side below [`MIN_SIDE`] or above [`MAX_SIDE`].
    #[error("an image side is between 16 and 7680 pixels")]
    SizeOutOfRange,
    /// A box of more pixels than a media worker decodes.
    #[error("a box holds at most 50 000 000 pixels")]
    BoxTooLarge,
    /// A mode after a single side rather than a box.
    #[error("fit, cover and exact follow a box such as 400x400")]
    ModeNeedsBox,
    /// A quality outside 1..=100.
    #[error("a quality is between 1 and 100")]
    QualityOutOfRange,
    /// A quality on PNG, which is lossless.
    #[error("png is lossless and takes no quality")]
    QualityOnLossless,
}

/// One rendition a rule makes: `kind/size/format`, with an optional `@quality` on a lossy still format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Output {
    /// An image made to `size`.
    Image {
        /// Its size.
        size: Size,
        /// Its format.
        format: ImageFormat,
        /// Its quality, 1..=100, when the rule names one.
        quality: Option<u8>,
    },
    /// A video `size` lines on its shorter side.
    Video {
        /// One of [`VIDEO_SIZES`].
        size: u32,
        /// Its container and codecs.
        format: VideoFormat,
    },
    /// One frame of a video as an image made to `size`.
    Poster {
        /// Its size.
        size: Size,
        /// Its format.
        format: ImageFormat,
        /// Its quality, 1..=100, when the rule names one.
        quality: Option<u8>,
    },
}

impl Output {
    /// The output spelled `name` (`image/1280/webp`, `image/400x400-cover/avif@60`, `video/720p/mp4`,
    /// `poster/640/jpeg`).
    ///
    /// # Errors
    /// [`OutputError`] naming what is wrong with the spelling.
    pub fn parse(name: &str) -> Result<Self, OutputError> {
        let mut parts = name.split('/');
        let (Some(kind), Some(size), Some(format), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(OutputError::Unknown);
        };
        match kind {
            "image" | "poster" => {
                let size = Size::parse(size)?;
                let (format, quality) = still(format)?;
                Ok(if kind == "image" {
                    Self::Image {
                        size,
                        format,
                        quality,
                    }
                } else {
                    Self::Poster {
                        size,
                        format,
                        quality,
                    }
                })
            }
            "video" => Ok(Self::Video {
                size: size
                    .strip_suffix('p')
                    .and_then(|lines| lines.parse::<u32>().ok())
                    .filter(|lines| VIDEO_SIZES.contains(lines))
                    .ok_or(OutputError::Unknown)?,
                format: VideoFormat::parse(format).ok_or(OutputError::Unknown)?,
            }),
            _ => Err(OutputError::Unknown),
        }
    }

    /// Its spelling in a rule.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Image {
                size,
                format,
                quality,
            } => format!(
                "image/{}/{}{}",
                size.name(),
                format.name(),
                quality_suffix(quality, "@")
            ),
            Self::Video { size, format } => format!("video/{size}p/{}", format.name()),
            Self::Poster {
                size,
                format,
                quality,
            } => format!(
                "poster/{}/{}{}",
                size.name(),
                format.name(),
                quality_suffix(quality, "@")
            ),
        }
    }

    /// The rendition's file name under the original's `.renditions/` folder: `image-<size>[-q<quality>].<ext>`, so a
    /// side-only output keeps the name it had before boxes and qualities existed.
    #[must_use]
    pub fn file_name(self) -> String {
        match self {
            Self::Image {
                size,
                format,
                quality,
            } => format!(
                "image-{}{}.{}",
                size.name(),
                quality_suffix(quality, "-q"),
                format.extension()
            ),
            Self::Video { size, format } => format!("video-{size}p.{}", format.name()),
            Self::Poster {
                size,
                format,
                quality,
            } => format!(
                "poster-{}{}.{}",
                size.name(),
                quality_suffix(quality, "-q"),
                format.extension()
            ),
        }
    }

    /// The family of content types it is made from: `image` for an image, `video` for a video or a poster.
    #[must_use]
    pub const fn source(self) -> &'static str {
        match self {
            Self::Image { .. } => "image",
            Self::Video { .. } | Self::Poster { .. } => "video",
        }
    }
}
