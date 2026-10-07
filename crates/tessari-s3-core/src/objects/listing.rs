//! Listing a bucket: the page a ListObjects request asks for, built from keys handed over in UTF-8 byte order by a
//! source that seeks. Prefix filtering, the delimiter roll-up (each CommonPrefix counts once against MaxKeys and its
//! whole range is skipped with one seek), the StartAfter filter, the one-item peek that decides IsTruncated, and the
//! position the next page resumes from.

/// What a page answers with: an object (with whatever the caller keeps for it) or a rolled-up prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry<T> {
    /// An object key.
    Object {
        /// The key.
        key: String,
        /// What the caller keeps with it.
        summary: T,
    },
    /// A CommonPrefix: the prefix up to and including the first delimiter after the request's prefix.
    Prefix(String),
}

/// Where a later page resumes: after a key, or after everything under a rolled-up prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// After this key.
    Key(String),
    /// After every key under this prefix.
    Prefix(String),
}

/// Where the source must start its next batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// At the first key strictly after this one.
    After(String),
    /// At this key itself if it exists, then the keys after it.
    At(String),
}

/// What the request asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSpec {
    /// Only keys starting with this.
    pub prefix: String,
    /// Roll keys up at the first occurrence of this after the prefix.
    pub delimiter: Option<String>,
    /// Most entries (objects plus prefixes) on the page.
    pub max_keys: usize,
    /// The client's StartAfter (V2) or Marker (V1): only entries strictly after it.
    pub start_after: Option<String>,
    /// Where a continuation token says the previous page ended; takes precedence over `start_after` for seeking.
    pub resume: Option<Resume>,
}

/// A finished page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    /// Entries in byte order.
    pub entries: Vec<Entry<T>>,
    /// Whether at least one more entry exists past the page.
    pub truncated: bool,
}

impl<T> Page<T> {
    /// Where the next page resumes, when the page is truncated.
    #[must_use]
    pub fn resume(&self) -> Option<Resume> {
        if !self.truncated {
            return None;
        }
        self.entries.last().map(|entry| match entry {
            Entry::Object { key, .. } => Resume::Key(key.clone()),
            Entry::Prefix(prefix) => Resume::Prefix(prefix.clone()),
        })
    }
}

/// A key that sorts after every key of at most 1,024 bytes starting with `prefix`, and before every key past them.
#[must_use]
pub fn past(prefix: &str) -> String {
    // U+10FFFF is the largest scalar value and its UTF-8 (F4 8F BF BF) the largest four bytes a key can hold; 256
    // of them are 1,024 bytes, longer than any key's remainder after a prefix.
    let mut anchor = String::with_capacity(prefix.len().saturating_add(1024));
    anchor.push_str(prefix);
    anchor.extend(std::iter::repeat_n('\u{10FFFF}', 256));
    anchor
}

/// The CommonPrefix `key` rolls up into, when there is a delimiter and the key holds it after `prefix`.
#[must_use]
pub fn rolled_up(prefix: &str, delimiter: Option<&str>, key: &str) -> Option<String> {
    let delimiter = delimiter.filter(|d| !d.is_empty())?;
    let rest = key.get(prefix.len()..)?;
    let at = rest.find(delimiter)?;
    let end = prefix.len().checked_add(at)?.checked_add(delimiter.len())?;
    key.get(..end).map(str::to_owned)
}

/// Builds one page from batches.
#[derive(Debug)]
pub struct Lister<T> {
    spec: ListSpec,
    entries: Vec<Entry<T>>,
    truncated: bool,
    anchor: Option<Anchor>,
}

impl<T> Lister<T> {
    /// A lister for `spec`, with its first anchor.
    #[must_use]
    pub fn new(spec: ListSpec) -> Self {
        let anchor = match (&spec.resume, &spec.start_after) {
            (Some(Resume::Key(key)), _) => Anchor::After(key.clone()),
            (Some(Resume::Prefix(prefix)), _) => Anchor::After(past(prefix)),
            (None, Some(after)) if after.as_str() >= spec.prefix.as_str() => {
                Anchor::After(after.clone())
            }
            _ => Anchor::At(spec.prefix.clone()),
        };
        Self {
            spec,
            entries: Vec::new(),
            truncated: false,
            anchor: Some(anchor),
        }
    }

    /// Where the next batch starts, or `None` when the page is complete.
    #[must_use]
    pub const fn anchor(&self) -> Option<&Anchor> {
        self.anchor.as_ref()
    }

    /// Takes the next batch, in byte order from the anchor; `more` says whether the source holds keys past it.
    pub fn feed(&mut self, batch: Vec<(String, T)>, more: bool) {
        let mut last_key = None;
        let mut skipping: Option<String> = None;
        for (key, summary) in batch {
            if skipping
                .as_deref()
                .is_some_and(|prefix| key.starts_with(prefix))
            {
                continue;
            }
            skipping = None;
            if !key.starts_with(&self.spec.prefix) {
                if key.as_str() > self.spec.prefix.as_str() {
                    // Past the prefix's range: nothing further can match.
                    self.anchor = None;
                    return;
                }
                last_key = Some(key);
                continue;
            }
            let entry = match rolled_up(&self.spec.prefix, self.spec.delimiter.as_deref(), &key) {
                Some(prefix) => {
                    skipping = Some(prefix.clone());
                    let not_past_start = self
                        .spec
                        .start_after
                        .as_deref()
                        .is_some_and(|after| prefix.as_str() <= after);
                    if not_past_start {
                        last_key = Some(key);
                        continue;
                    }
                    Entry::Prefix(prefix)
                }
                None => Entry::Object {
                    key: key.clone(),
                    summary,
                },
            };
            if self.entries.len() >= self.spec.max_keys {
                // One eligible entry past a full page is all IsTruncated needs.
                self.truncated = true;
                self.anchor = None;
                return;
            }
            self.entries.push(entry);
            last_key = Some(key);
        }
        self.anchor = match (more, skipping, last_key) {
            (false, _, _) | (true, None, None) => None,
            (true, Some(prefix), _) => Some(Anchor::After(past(&prefix))),
            (true, None, Some(key)) => Some(Anchor::After(key)),
        };
    }

    /// The page.
    #[must_use]
    pub fn finish(self) -> Page<T> {
        Page {
            entries: self.entries,
            truncated: self.truncated,
        }
    }
}

#[cfg(test)]
#[path = "listing_tests.rs"]
mod tests;
