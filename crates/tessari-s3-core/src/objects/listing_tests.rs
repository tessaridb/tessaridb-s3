use super::{Anchor, Entry, ListSpec, Lister, Page, Resume, past};

/// The keys of a bucket, sorted by byte order.
fn bucket(keys: &[&str]) -> Vec<String> {
    let mut keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
    keys.sort();
    keys
}

/// Drives a lister against `keys` the way the storage does: batches of `batch` keys from each anchor.
fn list(keys: &[String], spec: ListSpec, batch: usize) -> Page<()> {
    let mut lister = Lister::new(spec);
    let mut fetches = 0_u32;
    while let Some(anchor) = lister.anchor().cloned() {
        fetches = fetches.saturating_add(1);
        assert!(fetches < 1000, "the lister keeps asking");
        let from: Vec<&String> = keys
            .iter()
            .filter(|key| match &anchor {
                Anchor::After(after) => key.as_str() > after.as_str(),
                Anchor::At(at) => key.as_str() >= at.as_str(),
            })
            .collect();
        let taken: Vec<(String, ())> = from
            .iter()
            .take(batch)
            .map(|k| ((*k).clone(), ()))
            .collect();
        let more = from.len() > taken.len();
        lister.feed(taken, more);
    }
    lister.finish()
}

fn spec(prefix: &str, delimiter: Option<&str>, max_keys: usize) -> ListSpec {
    ListSpec {
        prefix: prefix.to_owned(),
        delimiter: delimiter.map(str::to_owned),
        max_keys,
        start_after: None,
        resume: None,
    }
}

fn names(page: &Page<()>) -> Vec<String> {
    page.entries
        .iter()
        .map(|entry| match entry {
            Entry::Object { key, .. } => key.clone(),
            Entry::Prefix(prefix) => format!("P:{prefix}"),
        })
        .collect()
}

#[test]
fn keys_come_in_byte_order_filtered_by_prefix() {
    let keys = bucket(&["b/1", "a", "b/2", "é", "Z", "b0", "€"]);
    let page = list(&keys, spec("", None, 1000), 2);
    assert_eq!(names(&page), ["Z", "a", "b/1", "b/2", "b0", "é", "€"]);
    assert!(!page.truncated);
    let under_b = list(&keys, spec("b/", None, 1000), 2);
    assert_eq!(names(&under_b), ["b/1", "b/2"]);
}

#[test]
fn a_delimiter_rolls_up_once_and_counts_once() {
    let keys = bucket(&["a/1", "a/2", "a/3", "b", "c/x/1", "c/y", "d"]);
    let page = list(&keys, spec("", Some("/"), 1000), 2);
    assert_eq!(names(&page), ["P:a/", "b", "P:c/", "d"]);
    let two = list(&keys, spec("", Some("/"), 2), 2);
    assert_eq!(names(&two), ["P:a/", "b"]);
    assert!(two.truncated, "c/ is eligible after the page");
    assert_eq!(two.resume(), Some(Resume::Key("b".to_owned())));
    let nested = list(&keys, spec("c/", Some("/"), 1000), 1);
    assert_eq!(names(&nested), ["P:c/x/", "c/y"]);
}

#[test]
fn a_page_ending_on_a_prefix_resumes_after_its_whole_range() {
    let keys = bucket(&["a/1", "a/2", "a/3", "b/1", "c"]);
    let first = list(&keys, spec("", Some("/"), 1), 1);
    assert_eq!(names(&first), ["P:a/"]);
    assert!(first.truncated);
    assert_eq!(first.resume(), Some(Resume::Prefix("a/".to_owned())));
    let mut next = spec("", Some("/"), 1000);
    next.resume = first.resume();
    assert_eq!(names(&list(&keys, next, 1)), ["P:b/", "c"]);
}

#[test]
fn start_after_is_exclusive_and_filters_prefixes_not_past_it() {
    let keys = bucket(&["a/1", "a/2", "b", "c"]);
    let mut after = spec("", Some("/"), 1000);
    after.start_after = Some("a/1".to_owned());
    assert_eq!(
        names(&list(&keys, after, 2)),
        ["b", "c"],
        "a/ is not greater than a/1"
    );
    let mut plain = spec("", None, 1000);
    plain.start_after = Some("a/1".to_owned());
    assert_eq!(names(&list(&keys, plain, 2)), ["a/2", "b", "c"]);
}

#[test]
fn a_full_page_is_truncated_only_when_another_entry_exists() {
    let keys = bucket(&["a", "b", "c"]);
    let exact = list(&keys, spec("", None, 3), 10);
    assert!(!exact.truncated, "nothing after c");
    assert_eq!(exact.resume(), None);
    let short = list(&keys, spec("", None, 2), 10);
    assert!(short.truncated);
    assert_eq!(short.resume(), Some(Resume::Key("b".to_owned())));
    let zero = list(&keys, spec("", None, 0), 10);
    assert!(zero.entries.is_empty() && zero.truncated);
    let none = list(&bucket(&[]), spec("", None, 0), 10);
    assert!(none.entries.is_empty() && !none.truncated);
}

#[test]
fn a_prefix_start_includes_a_key_equal_to_the_prefix() {
    let keys = bucket(&["a", "ab", "abc", "b"]);
    let page = list(&keys, spec("ab", None, 1000), 1);
    assert_eq!(names(&page), ["ab", "abc"]);
    assert_eq!(
        Lister::<()>::new(spec("ab", None, 10)).anchor(),
        Some(&Anchor::At("ab".to_owned()))
    );
}

#[test]
fn past_sorts_after_every_key_under_the_prefix_and_before_the_next() {
    let skip = past("a/");
    let longest_under = format!("a/{}", "\u{10FFFF}".repeat(255));
    assert!(longest_under.len() <= 1024);
    assert!(skip.as_str() > longest_under.as_str());
    assert!(skip.as_str() > "a/zzzz");
    assert!(skip.as_str() < "a0");
    assert!(skip.as_str() < "b");
}

/// The listing S3 describes, computed directly: every key under the prefix, rolled up at the first delimiter after
/// it, each prefix once, in byte order.
fn reference(keys: &[String], prefix: &str, delimiter: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in keys.iter().filter(|k| k.starts_with(prefix)) {
        let rolled = delimiter.and_then(|d| {
            key[prefix.len()..].find(d).map(|at| {
                format!(
                    "P:{}",
                    &key[..prefix.len().saturating_add(at).saturating_add(d.len())]
                )
            })
        });
        let name = rolled.unwrap_or_else(|| key.clone());
        if out.last() != Some(&name) {
            out.push(name);
        }
    }
    out
}

/// Keys from a fixed pseudo-random sequence over an alphabet that mixes ASCII, the delimiter and multi-byte UTF-8.
fn generated(count: usize, seed: u64) -> Vec<String> {
    const ALPHABET: [&str; 8] = ["a", "b", "/", "é", "Z", "€", "0", "-"];
    let mut state = seed;
    let mut keys = Vec::new();
    for _ in 0..count {
        let mut key = String::new();
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let length = usize::try_from((state >> 60).wrapping_rem(6))
            .expect("small")
            .wrapping_add(1);
        for _ in 0..length {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            key.push_str(
                ALPHABET[usize::try_from(state >> 59)
                    .expect("small")
                    .checked_rem(ALPHABET.len())
                    .expect("non-empty alphabet")],
            );
        }
        keys.push(key);
    }
    keys.sort();
    keys.dedup();
    keys
}

#[test]
fn paging_with_any_page_size_concatenates_to_the_reference_listing() {
    for seed in 1..=20 {
        let keys = generated(60, seed);
        for (prefix, delimiter) in [
            ("", None),
            ("", Some("/")),
            ("a", Some("/")),
            ("a/", Some("/")),
            ("b", None),
        ] {
            let expected = reference(&keys, prefix, delimiter);
            if seed == 1 && delimiter.is_some() && prefix.is_empty() {
                assert!(
                    expected.iter().any(|name| name.starts_with("P:")),
                    "control: the reference rolls something up"
                );
            }
            for max_keys in 1..=6 {
                let mut seen = Vec::new();
                let mut resume = None;
                for pages in 0.. {
                    assert!(pages <= keys.len(), "paging does not advance: {seen:?}");
                    let mut page_spec = spec(prefix, delimiter, max_keys);
                    page_spec.resume = resume.clone();
                    let page = list(&keys, page_spec, 4);
                    assert!(page.entries.len() <= max_keys);
                    seen.extend(names(&page));
                    match page.resume() {
                        Some(next) => resume = Some(next),
                        None => break,
                    }
                }
                assert_eq!(
                    seen, expected,
                    "seed {seed} prefix {prefix:?} delimiter {delimiter:?} max {max_keys}"
                );
            }
        }
    }
}
