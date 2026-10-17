//! IAM wildcard matching: `*` any run of characters (none included), `?` exactly one; everything else literal.

/// Whether `text` matches `pattern`, comparing characters as given.
#[must_use]
pub(super) fn matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut at_pattern, mut at_text) = (0, 0);
    // The last `*` seen and the text position it was tried at; a mismatch retries it one character further.
    let mut star: Option<(usize, usize)> = None;
    while at_text < text.len() {
        match pattern.get(at_pattern) {
            Some('*') => {
                star = Some((at_pattern, at_text));
                at_pattern = at_pattern.saturating_add(1);
            }
            Some(&wanted) if wanted == '?' || Some(&wanted) == text.get(at_text) => {
                at_pattern = at_pattern.saturating_add(1);
                at_text = at_text.saturating_add(1);
            }
            _ => match star {
                Some((star_at, tried)) => {
                    let next = tried.saturating_add(1);
                    at_pattern = star_at.saturating_add(1);
                    at_text = next;
                    star = Some((star_at, next));
                }
                None => return false,
            },
        }
    }
    pattern.iter().skip(at_pattern).all(|c| *c == '*')
}

/// Whether `text` matches `pattern` ignoring ASCII case — action names are case-insensitive.
#[must_use]
pub(super) fn matches_ignoring_case(pattern: &str, text: &str) -> bool {
    matches(&pattern.to_ascii_lowercase(), &text.to_ascii_lowercase())
}
