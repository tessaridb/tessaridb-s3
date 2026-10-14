//! Media rules per bucket (G004): which objects — by content type and key prefix — get which renditions, derived from
//! the original and written under `<key>.renditions/`. A rendition never matches a rule.

/// The segment every rendition key carries; a key holding it is never matched.
pub const RENDITIONS: &str = ".renditions/";

/// Most rules one bucket may hold.
pub const MAX_RULES: usize = 100;

/// A named output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// An image at most 320 px on its longer side.
    Thumb,
    /// An image at most 2048 px on its longer side.
    Web,
    /// A video at most 720 lines high.
    Video720,
    /// One frame of a video as an image.
    Poster,
}

impl Preset {
    const ALL: [Self; 4] = [Self::Thumb, Self::Web, Self::Video720, Self::Poster];

    /// The preset named `name`, if any.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.name() == name)
    }

    /// Its name, as a rule and a rendition key spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Thumb => "thumb",
            Self::Web => "web",
            Self::Video720 => "720p",
            Self::Poster => "poster",
        }
    }

    /// Whether it is made from an image (else from a video).
    #[must_use]
    pub const fn is_image(self) -> bool {
        matches!(self, Self::Thumb | Self::Web)
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
    pub outputs: Vec<Preset>,
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
    /// An image output from video types, or a video output from image types.
    #[error("the output '{0}' cannot be made from every content type the rule takes")]
    WrongKind(&'static str),
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
        for output in &rule.outputs {
            let family = if output.is_image() { "image" } else { "video" };
            let fits = rule.content_types.iter().all(|pattern| {
                pattern
                    .to_ascii_lowercase()
                    .starts_with(&format!("{family}/"))
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
