//! ListMultipartUploads: one page of a bucket's open uploads, by key in UTF-8 byte order and, within a key, by
//! initiation, from a source that seeks. Unlike an object listing a key may contribute several entries, so the page
//! resumes after an upload rather than after a key. Each upload counts once against max-uploads, each CommonPrefix
//! counts once and its whole range is skipped with one seek, and a CommonPrefix is shown only past the key-marker.

use super::listing::rolled_up;

/// What the request asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadSpec {
    /// Only keys starting with this.
    pub prefix: String,
    /// Roll keys up at the first occurrence of this after the prefix.
    pub delimiter: Option<String>,
    /// Most entries (uploads plus prefixes) on the page.
    pub max_uploads: usize,
    /// The client's key-marker.
    pub key_marker: Option<String>,
}

/// The client's upload-id-marker, resolved against the open uploads of the key-marker's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marker<T> {
    /// No upload-id-marker (or no key-marker, which makes S3 ignore it).
    None,
    /// The marker names this open upload of the key-marker's key.
    Found(T),
    /// The marker names no open upload of that key (completed or aborted since the previous page).
    Gone,
}

/// Where the source must start its next batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadAnchor<T> {
    /// At the first upload of the first key at or after this one.
    From(String),
    /// After every upload of this key.
    PastKey(String),
    /// After every upload of every key starting with this prefix.
    PastPrefix(String),
    /// After this upload.
    After(T),
}

/// What a page answers with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadEntry<T> {
    /// An open upload.
    Upload {
        /// Its key.
        key: String,
        /// What the caller keeps with it.
        item: T,
    },
    /// A CommonPrefix: the prefix up to and including the first delimiter after the request's prefix.
    Prefix(String),
}

/// A finished page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPage<T> {
    /// Entries in order.
    pub entries: Vec<UploadEntry<T>>,
    /// Whether at least one more entry exists past the page.
    pub truncated: bool,
}

/// Builds one page from batches.
#[derive(Debug)]
pub struct UploadLister<T> {
    spec: UploadSpec,
    entries: Vec<UploadEntry<T>>,
    truncated: bool,
    anchor: Option<UploadAnchor<T>>,
}

impl<T: Clone> UploadLister<T> {
    /// A lister for `spec`. The first anchor follows S3's markers: none → the prefix; a key-marker alone → past every
    /// upload of that key; with an upload-id-marker → after that upload, or — when it is gone — at its key again,
    /// which may repeat an upload the previous page showed but never skips one.
    #[must_use]
    pub fn new(spec: UploadSpec, marker: Marker<T>) -> Self {
        let anchor = match spec.key_marker.as_deref() {
            Some(key) if key >= spec.prefix.as_str() => match marker {
                Marker::None => UploadAnchor::PastKey(key.to_owned()),
                Marker::Found(item) => UploadAnchor::After(item),
                Marker::Gone => UploadAnchor::From(key.to_owned()),
            },
            _ => UploadAnchor::From(spec.prefix.clone()),
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
    pub const fn anchor(&self) -> Option<&UploadAnchor<T>> {
        self.anchor.as_ref()
    }

    /// Takes the next batch, in order from the anchor; `more` says whether the source holds uploads past it.
    pub fn feed(&mut self, batch: Vec<(String, T)>, more: bool) {
        let mut last = None;
        let mut skipping: Option<String> = None;
        for (key, item) in batch {
            if skipping
                .as_deref()
                .is_some_and(|prefix| key.starts_with(prefix))
            {
                continue;
            }
            skipping = None;
            if !key.starts_with(&self.spec.prefix) {
                // Past the prefix's range: nothing further can match.
                self.anchor = None;
                return;
            }
            let entry = match rolled_up(&self.spec.prefix, self.spec.delimiter.as_deref(), &key) {
                Some(prefix) => {
                    let not_past_marker = self
                        .spec
                        .key_marker
                        .as_deref()
                        .is_some_and(|marker| prefix.as_str() <= marker);
                    skipping = Some(prefix.clone());
                    if not_past_marker {
                        continue;
                    }
                    UploadEntry::Prefix(prefix)
                }
                None => UploadEntry::Upload {
                    key,
                    item: item.clone(),
                },
            };
            if self.entries.len() >= self.spec.max_uploads {
                // One eligible entry past a full page is all IsTruncated needs.
                self.truncated = true;
                self.anchor = None;
                return;
            }
            self.entries.push(entry);
            last = Some(item);
        }
        self.anchor = match (more, skipping, last) {
            (false, _, _) | (true, None, None) => None,
            (true, Some(prefix), _) => Some(UploadAnchor::PastPrefix(prefix)),
            (true, None, Some(item)) => Some(UploadAnchor::After(item)),
        };
    }

    /// The page.
    #[must_use]
    pub fn finish(self) -> UploadPage<T> {
        UploadPage {
            entries: self.entries,
            truncated: self.truncated,
        }
    }
}

#[cfg(test)]
#[path = "upload_listing_tests.rs"]
mod tests;
