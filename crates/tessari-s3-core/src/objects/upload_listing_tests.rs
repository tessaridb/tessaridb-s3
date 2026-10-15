use super::{Marker, UploadAnchor, UploadEntry, UploadLister, UploadPage, UploadSpec};

/// An open upload as the store orders them: its key, then when it began.
type Upload = (String, u32);

/// The uploads of a bucket in the store's order: key bytes, then initiation.
fn store(uploads: &[(&str, u32)]) -> Vec<Upload> {
    let mut uploads: Vec<Upload> = uploads.iter().map(|(k, n)| ((*k).to_owned(), *n)).collect();
    uploads.sort();
    uploads
}

/// Drives a lister against `uploads` as the storage does: batches of `batch` from each anchor. Returns the page and
/// the anchors asked for.
fn list(
    uploads: &[Upload],
    spec: UploadSpec,
    marker: Marker<Upload>,
    batch: usize,
) -> (UploadPage<Upload>, Vec<UploadAnchor<Upload>>) {
    let mut lister = UploadLister::new(spec, marker);
    let mut anchors = Vec::new();
    while let Some(anchor) = lister.anchor().cloned() {
        assert!(anchors.len() < 1000, "the lister keeps asking");
        let from: Vec<&Upload> = uploads
            .iter()
            .filter(|(key, at)| match &anchor {
                UploadAnchor::From(from) => key.as_str() >= from.as_str(),
                UploadAnchor::PastKey(past) => key.as_str() > past.as_str(),
                UploadAnchor::PastPrefix(prefix) => {
                    key.as_str() > prefix.as_str() && !key.starts_with(prefix.as_str())
                }
                UploadAnchor::After((after_key, after_at)) => (key, at) > (after_key, after_at),
            })
            .collect();
        let taken: Vec<(String, Upload)> = from
            .iter()
            .take(batch)
            .map(|upload| (upload.0.clone(), (*upload).clone()))
            .collect();
        let more = from.len() > taken.len();
        anchors.push(anchor);
        lister.feed(taken, more);
    }
    (lister.finish(), anchors)
}

fn spec(prefix: &str, delimiter: Option<&str>, max: usize, key_marker: Option<&str>) -> UploadSpec {
    UploadSpec {
        prefix: prefix.to_owned(),
        delimiter: delimiter.map(str::to_owned),
        max_uploads: max,
        key_marker: key_marker.map(str::to_owned),
    }
}

fn names(page: &UploadPage<Upload>) -> Vec<String> {
    page.entries
        .iter()
        .map(|entry| match entry {
            UploadEntry::Upload { key, item } => format!("{key}#{}", item.1),
            UploadEntry::Prefix(prefix) => format!("P:{prefix}"),
        })
        .collect()
}

#[test]
fn uploads_come_by_key_then_initiation_and_each_counts_once() {
    let uploads = store(&[("b", 1), ("a", 2), ("a/x", 1), ("a", 1), ("é", 1)]);
    let (page, _) = list(&uploads, spec("", None, 3, None), Marker::None, 2);
    assert_eq!(names(&page), ["a#1", "a#2", "a/x#1"]);
    assert!(page.truncated);
    let (page, _) = list(&uploads, spec("a", None, 1000, None), Marker::None, 2);
    assert_eq!(names(&page), ["a#1", "a#2", "a/x#1"]);
    assert!(!page.truncated);
}

#[test]
fn paging_by_the_last_entry_neither_repeats_nor_skips() {
    let uploads = store(&[
        ("a", 1),
        ("a", 2),
        ("a", 3),
        ("a/b", 1),
        ("a\u{0}", 1),
        ("ab", 1),
        ("ab", 2),
        ("b/c/d", 1),
        ("b/c/e", 1),
        ("b/f", 1),
        ("€", 1),
    ]);
    for delimiter in [None, Some("/")] {
        for max in [1, 2, 3] {
            for batch in [1, 2, 5] {
                let mut seen = Vec::new();
                let mut key_marker: Option<String> = None;
                let mut marker = Marker::None;
                loop {
                    let request = spec("", delimiter, max, key_marker.as_deref());
                    let (page, _) = list(&uploads, request, marker, batch);
                    seen.extend(names(&page));
                    // A pager that stops advancing must fail here rather than loop.
                    assert!(
                        seen.len() <= uploads.len(),
                        "the pages do not end: {seen:?}"
                    );
                    if !page.truncated {
                        break;
                    }
                    let last = page.entries.last().cloned();
                    (key_marker, marker) = match last {
                        Some(UploadEntry::Upload { key, item }) => (Some(key), Marker::Found(item)),
                        Some(UploadEntry::Prefix(prefix)) => (Some(prefix), Marker::None),
                        None => panic!("a truncated page with no entry"),
                    };
                }
                let expected: Vec<String> = match delimiter {
                    None => uploads.iter().map(|(k, n)| format!("{k}#{n}")).collect(),
                    Some(_) => [
                        "a#1", "a#2", "a#3", "a\u{0}#1", "P:a/", "ab#1", "ab#2", "P:b/", "€#1",
                    ]
                    .map(str::to_owned)
                    .to_vec(),
                };
                assert_eq!(
                    seen, expected,
                    "delimiter {delimiter:?} max {max} batch {batch}"
                );
            }
        }
    }
}

#[test]
fn a_rolled_up_prefix_counts_once_and_its_range_is_skipped_with_one_seek() {
    let mut many: Vec<(String, u32)> = (0..50).map(|i| (format!("a/{i:02}"), 1)).collect();
    many.push(("b".to_owned(), 1));
    let refs: Vec<(&str, u32)> = many.iter().map(|(k, n)| (k.as_str(), *n)).collect();
    let uploads = store(&refs);
    let (page, anchors) = list(&uploads, spec("", Some("/"), 2, None), Marker::None, 3);
    assert_eq!(names(&page), ["P:a/", "b#1"]);
    assert!(!page.truncated);
    assert_eq!(
        anchors,
        [
            UploadAnchor::From(String::new()),
            UploadAnchor::PastPrefix("a/".to_owned())
        ]
    );
}

#[test]
fn a_key_marker_alone_starts_past_every_upload_of_that_key() {
    let uploads = store(&[("a", 1), ("a", 2), ("a0", 1), ("b", 1)]);
    let (page, anchors) = list(&uploads, spec("", None, 10, Some("a")), Marker::None, 10);
    assert_eq!(names(&page), ["a0#1", "b#1"]);
    assert_eq!(anchors[0], UploadAnchor::PastKey("a".to_owned()));
    // A key-marker before the prefix starts at the prefix.
    let (page, _) = list(&uploads, spec("b", None, 10, Some("a")), Marker::None, 10);
    assert_eq!(names(&page), ["b#1"]);
}

#[test]
fn an_upload_marker_resumes_inside_its_key() {
    let uploads = store(&[("a", 1), ("a", 2), ("a", 3), ("b", 1)]);
    let found = Marker::Found(("a".to_owned(), 2));
    let (page, anchors) = list(&uploads, spec("", None, 10, Some("a")), found, 10);
    assert_eq!(names(&page), ["a#3", "b#1"]);
    assert_eq!(anchors[0], UploadAnchor::After(("a".to_owned(), 2)));
}

#[test]
fn a_marker_whose_upload_is_gone_restarts_at_its_key_rather_than_skipping() {
    let uploads = store(&[("a", 1), ("a", 3), ("b", 1)]);
    let (page, anchors) = list(&uploads, spec("", None, 10, Some("a")), Marker::Gone, 10);
    assert_eq!(names(&page), ["a#1", "a#3", "b#1"]);
    assert_eq!(anchors[0], UploadAnchor::From("a".to_owned()));
}

#[test]
fn a_prefix_not_past_the_key_marker_is_not_shown() {
    let uploads = store(&[("a/1", 1), ("a/2", 1), ("b", 1)]);
    let (page, _) = list(
        &uploads,
        spec("", Some("/"), 10, Some("a/1")),
        Marker::None,
        10,
    );
    assert_eq!(names(&page), ["b#1"]);
}

#[test]
fn a_page_of_none_is_empty_and_truncated_when_anything_is_left() {
    let uploads = store(&[("a", 1)]);
    let (page, _) = list(&uploads, spec("", None, 0, None), Marker::None, 10);
    assert!(page.entries.is_empty());
    assert!(page.truncated);
    let (page, _) = list(&[], spec("", None, 0, None), Marker::None, 10);
    assert!(!page.truncated);
}
