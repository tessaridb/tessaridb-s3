//! A bucket's CORS rules (S3 user guide, "Elements of a CORS configuration", retrieved 2026-10-17): what a rule may
//! say, and which rule — the first that matches — answers a browser's preflight or its actual request.

/// Most rules one bucket may hold.
pub const MAX_RULES: usize = 100;
/// Longest rule ID, in characters.
pub const MAX_ID_CHARS: usize = 255;

/// A method a rule may allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`.
    Get,
    /// `PUT`.
    Put,
    /// `POST`.
    Post,
    /// `DELETE`.
    Delete,
    /// `HEAD`.
    Head,
}

impl Method {
    /// The method `name` names, exactly as S3 spells it.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "GET" => Self::Get,
            "PUT" => Self::Put,
            "POST" => Self::Post,
            "DELETE" => Self::Delete,
            "HEAD" => Self::Head,
            _ => return None,
        })
    }

    /// The method as S3 spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Put => "PUT",
            Self::Post => "POST",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
        }
    }
}

/// One rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The rule's ID.
    pub id: Option<String>,
    /// Origins allowed, each with at most one `*`.
    pub origins: Vec<String>,
    /// Methods allowed.
    pub methods: Vec<Method>,
    /// Request headers a preflight may ask for, each with at most one `*`.
    pub headers: Vec<String>,
    /// Response headers a browser may show a script.
    pub expose: Vec<String>,
    /// How long a browser may keep a preflight's answer, in seconds.
    pub max_age: Option<u32>,
}

/// Why a configuration is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CorsError {
    /// No rule at all.
    #[error("a CORS configuration holds at least one rule")]
    NoRules,
    /// More than [`MAX_RULES`].
    #[error("a CORS configuration holds at most 100 rules")]
    TooManyRules,
    /// A rule allows no origin.
    #[error("a CORS rule names at least one AllowedOrigin")]
    NoOrigin,
    /// A rule allows no method.
    #[error("a CORS rule names at least one AllowedMethod")]
    NoMethod,
    /// An origin or header pattern with more than one `*`.
    #[error("an AllowedOrigin or AllowedHeader holds at most one * wildcard")]
    TwoWildcards,
    /// A wildcard in an exposed header.
    #[error("an ExposeHeader cannot hold a * wildcard")]
    WildcardExposed,
    /// An ID longer than [`MAX_ID_CHARS`].
    #[error("a rule ID is at most 255 characters")]
    LongId,
}

/// Whether `rules` may be stored.
///
/// # Errors
/// [`CorsError`] naming the first fault.
pub fn validate(rules: &[Rule]) -> Result<(), CorsError> {
    if rules.is_empty() {
        return Err(CorsError::NoRules);
    }
    if rules.len() > MAX_RULES {
        return Err(CorsError::TooManyRules);
    }
    let wildcards = |pattern: &String| pattern.matches('*').count();
    for rule in rules {
        if rule
            .id
            .as_ref()
            .is_some_and(|id| id.chars().count() > MAX_ID_CHARS)
        {
            return Err(CorsError::LongId);
        }
        if rule.origins.is_empty() {
            return Err(CorsError::NoOrigin);
        }
        if rule.methods.is_empty() {
            return Err(CorsError::NoMethod);
        }
        if rule
            .origins
            .iter()
            .chain(&rule.headers)
            .any(|p| wildcards(p) > 1)
        {
            return Err(CorsError::TwoWildcards);
        }
        if rule.expose.iter().any(|header| header.contains('*')) {
            return Err(CorsError::WildcardExposed);
        }
    }
    Ok(())
}

/// Whether `value` fits `pattern`, whose one `*` stands for any run of characters.
fn fits(pattern: &str, value: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == value,
        Some((head, tail)) => {
            value.len() >= head.len().saturating_add(tail.len())
                && value.starts_with(head)
                && value.ends_with(tail)
        }
    }
}

fn allows(rule: &Rule, origin: &str, method: &str) -> bool {
    rule.origins.iter().any(|pattern| fits(pattern, origin))
        && rule
            .methods
            .iter()
            .any(|allowed| allowed.as_str() == method)
}

/// The rule answering a preflight from `origin` for `method` asking for `headers`.
#[must_use]
pub fn preflight<'r>(
    rules: &'r [Rule],
    origin: &str,
    method: &str,
    headers: &[&str],
) -> Option<&'r Rule> {
    rules.iter().find(|rule| {
        allows(rule, origin, method)
            && headers.iter().all(|asked| {
                let asked = asked.to_ascii_lowercase();
                rule.headers
                    .iter()
                    .any(|pattern| fits(&pattern.to_ascii_lowercase(), &asked))
            })
    })
}

/// The rule covering an actual request from `origin` with `method`.
#[must_use]
pub fn actual<'r>(rules: &'r [Rule], origin: &str, method: &str) -> Option<&'r Rule> {
    rules.iter().find(|rule| allows(rule, origin, method))
}

/// The `Access-Control-Allow-Origin` value `rule` answers `origin` with.
#[must_use]
pub fn allowed_origin<'a>(rule: &Rule, origin: &'a str) -> &'a str {
    match rule.origins.iter().find(|pattern| fits(pattern, origin)) {
        Some(pattern) if pattern == "*" => "*",
        _ => origin,
    }
}
