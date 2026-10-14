//! Media rules per bucket (G004): which objects — by content type and key prefix — get which renditions, derived from
//! the original and written under `<key>.renditions/`. A rendition never matches a rule.

/// The segment every rendition key carries; a key holding it is never matched.
pub const RENDITIONS: &str = ".renditions/";

/// Most rules one bucket may hold.
pub const MAX_RULES: usize = 100;

/// The long sides, in pixels, an image or a poster is fitted into: thumbnails, the common responsive widths, full HD and
/// up to 4K.
pub const IMAGE_SIZES: [u32; 10] = [150, 320, 640, 768, 1024, 1280, 1440, 1920, 2560, 3840];

/// The heights a video is made at, spelled `<size>p` — the shorter side, so a portrait video keeps its shape.
pub const VIDEO_SIZES: [u32; 6] = [360, 480, 720, 1080, 1440, 2160];

/// The still formats an image or a poster is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImageFormat {
    /// JPEG: every viewer reads it.
    Jpeg,
    /// PNG: lossless, keeps transparency.
    Png,
    /// WebP: smaller than JPEG at the same quality, keeps transparency.
    Webp,
    /// AVIF: smaller still, slower to encode.
    Avif,
}

impl ImageFormat {
    const ALL: [Self; 4] = [Self::Jpeg, Self::Png, Self::Webp, Self::Avif];

    /// Its name in an output.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Avif => "avif",
        }
    }

    /// Its file extension.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Avif => "avif",
        }
    }

    /// The `Content-Type` its file is stored with.
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
            Self::Avif => "image/avif",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.name() == name)
    }
}

/// The containers a video is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VideoFormat {
    /// MP4 with H.264 and AAC: plays everywhere.
    Mp4,
    /// WebM with VP9 and Opus: open and smaller.
    Webm,
}

impl VideoFormat {
    const ALL: [Self; 2] = [Self::Mp4, Self::Webm];

    /// Its name in an output, which is also its file extension.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
        }
    }

    /// The `Content-Type` its file is stored with.
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Mp4 => "video/mp4",
            Self::Webm => "video/webm",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.name() == name)
    }
}

/// One rendition a rule makes: `kind/size/format`, each from its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Output {
    /// An image fitted into `size` pixels on its long side.
    Image {
        /// One of [`IMAGE_SIZES`].
        size: u32,
        /// Its format.
        format: ImageFormat,
    },
    /// A video `size` lines on its shorter side.
    Video {
        /// One of [`VIDEO_SIZES`].
        size: u32,
        /// Its container and codecs.
        format: VideoFormat,
    },
    /// One frame of a video as an image fitted into `size` pixels.
    Poster {
        /// One of [`IMAGE_SIZES`].
        size: u32,
        /// Its format.
        format: ImageFormat,
    },
}

impl Output {
    /// The output spelled `name` (`image/1280/webp`, `video/720p/mp4`, `poster/640/jpeg`), if it is one.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        let mut parts = name.split('/');
        let (kind, size, format) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let image_size = || {
            size.parse::<u32>()
                .ok()
                .filter(|size| IMAGE_SIZES.contains(size))
        };
        match kind {
            "image" => Some(Self::Image {
                size: image_size()?,
                format: ImageFormat::parse(format)?,
            }),
            "poster" => Some(Self::Poster {
                size: image_size()?,
                format: ImageFormat::parse(format)?,
            }),
            "video" => Some(Self::Video {
                size: size
                    .strip_suffix('p')?
                    .parse::<u32>()
                    .ok()
                    .filter(|size| VIDEO_SIZES.contains(size))?,
                format: VideoFormat::parse(format)?,
            }),
            _ => None,
        }
    }

    /// Its spelling in a rule.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Image { size, format } => format!("image/{size}/{}", format.name()),
            Self::Video { size, format } => format!("video/{size}p/{}", format.name()),
            Self::Poster { size, format } => format!("poster/{size}/{}", format.name()),
        }
    }

    /// The rendition's file name under the original's `.renditions/` folder.
    #[must_use]
    pub fn file_name(self) -> String {
        match self {
            Self::Image { size, format } => format!("image-{size}.{}", format.extension()),
            Self::Video { size, format } => format!("video-{size}p.{}", format.name()),
            Self::Poster { size, format } => format!("poster-{size}.{}", format.extension()),
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

/// One rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The rule's ID.
    pub id: String,
    /// Content types it takes: `type/subtype` or `type/*`.
    pub content_types: Vec<String>,
    /// Only keys starting with this; empty for every key.
    pub prefix: String,
    /// The renditions it makes.
    pub outputs: Vec<Output>,
}

/// Why a set of rules is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    /// A rule taking no content type.
    #[error("a media rule names at least one content type")]
    NoContentType,
    /// A rule making nothing.
    #[error("a media rule names at least one output")]
    NoOutput,
    /// A content type that is not `type/subtype` or `type/*`.
    #[error("'{0}' is not a content type: type/subtype or type/*")]
    BadContentType(String),
    /// An image output from video types, or a video or poster output from image types.
    #[error("the output '{0}' cannot be made from every content type the rule takes")]
    WrongKind(String),
    /// One output named twice in a rule.
    #[error("a media rule names each output once")]
    RepeatedOutput,
    /// Two rules with one ID.
    #[error("media rule IDs must be unique")]
    DuplicateId,
    /// A prefix inside renditions, which no rule may take.
    #[error("a media rule cannot take keys under a '.renditions/' segment")]
    RenditionPrefix,
    /// More than [`MAX_RULES`].
    #[error("a bucket holds at most 100 media rules")]
    TooManyRules,
}

/// The `type/subtype` of a `Content-Type` value, lowercased, its parameters dropped.
fn essence(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Whether `pattern` (`type/subtype` or `type/*`) takes `kind` (an essence).
fn takes(pattern: &str, kind: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    match pattern.strip_suffix("/*") {
        Some(family) => kind.split_once('/').is_some_and(|(top, _)| top == family),
        None => pattern == kind,
    }
}

fn well_formed(pattern: &str) -> bool {
    let token = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&byte))
    };
    pattern
        .split_once('/')
        .is_some_and(|(top, sub)| token(top) && (sub == "*" || token(sub)))
}

/// Checks `rules` before they are stored. An empty list is valid: it turns media processing off.
///
/// # Errors
/// The first [`MediaError`] found.
pub fn validate(rules: &[Rule]) -> Result<(), MediaError> {
    if rules.len() > MAX_RULES {
        return Err(MediaError::TooManyRules);
    }
    let mut ids = std::collections::BTreeSet::new();
    for rule in rules {
        if !ids.insert(rule.id.as_str()) {
            return Err(MediaError::DuplicateId);
        }
        if rule.content_types.is_empty() {
            return Err(MediaError::NoContentType);
        }
        if rule.outputs.is_empty() {
            return Err(MediaError::NoOutput);
        }
        if let Some(bad) = rule
            .content_types
            .iter()
            .find(|pattern| !well_formed(pattern))
        {
            return Err(MediaError::BadContentType(bad.clone()));
        }
        let mut named = std::collections::BTreeSet::new();
        for output in &rule.outputs {
            if !named.insert(*output) {
                return Err(MediaError::RepeatedOutput);
            }
            let fits = rule.content_types.iter().all(|pattern| {
                pattern
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}/", output.source()))
            });
            if !fits {
                return Err(MediaError::WrongKind(output.name()));
            }
        }
        if rule.prefix.contains(RENDITIONS) {
            return Err(MediaError::RenditionPrefix);
        }
    }
    Ok(())
}

/// Whether `rule` takes an object of `content_type` at `key`; a rendition key is never taken.
#[must_use]
pub fn matches(rule: &Rule, content_type: &str, key: &str) -> bool {
    let kind = essence(content_type);
    !key.contains(RENDITIONS)
        && key.starts_with(&rule.prefix)
        && rule
            .content_types
            .iter()
            .any(|pattern| takes(pattern, &kind))
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
