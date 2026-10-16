use super::{BUCKET_TAGS, OBJECT_TAGS, TagError, TagSet};

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn keys(set: &TagSet) -> Vec<&str> {
    set.tags().iter().map(|tag| tag.key.as_str()).collect()
}

#[test]
fn a_tag_set_keeps_its_tags_in_order_and_refuses_what_s3_refuses() {
    let set = TagSet::new(pairs(&[("team", "blue"), ("tier", "")]), OBJECT_TAGS).expect("valid");
    assert_eq!(keys(&set), ["team", "tier"]);
    assert_eq!(set.tags()[1].value, "");
    let allowed = "Aé 9+-=._:/@";
    assert!(TagSet::new(pairs(&[(allowed, allowed)]), OBJECT_TAGS).is_ok());
    for (list, refusal) in [
        (vec![("a", "1"), ("a", "2")], TagError::DuplicateKey),
        (vec![("", "x")], TagError::KeyLength),
        (vec![("aws:owner", "x")], TagError::Reserved),
        (vec![("a#b", "x")], TagError::Character),
        (vec![("a", "x;y")], TagError::Character),
    ] {
        assert_eq!(
            TagSet::new(pairs(&list), OBJECT_TAGS).err(),
            Some(refusal),
            "{list:?}"
        );
    }
    let key = "k".repeat(128);
    let value = "v".repeat(256);
    assert!(TagSet::new(pairs(&[(&key, &value)]), OBJECT_TAGS).is_ok());
    let longer_key = format!("{key}k");
    let longer_value = format!("{value}v");
    assert_eq!(
        TagSet::new(pairs(&[(&longer_key, "x")]), OBJECT_TAGS).err(),
        Some(TagError::KeyLength)
    );
    assert_eq!(
        TagSet::new(pairs(&[("k", &longer_value)]), OBJECT_TAGS).err(),
        Some(TagError::ValueLength)
    );
    // Lengths are characters, not bytes: 128 two-byte characters are a valid key.
    assert!(TagSet::new(pairs(&[(&"é".repeat(128), "x")]), OBJECT_TAGS).is_ok());
}

#[test]
fn an_object_holds_ten_tags_and_a_bucket_fifty() {
    let names: Vec<String> = (0..51).map(|n| format!("k{n}")).collect();
    let first = |count: usize| -> Vec<(String, String)> {
        names[..count]
            .iter()
            .map(|key| (key.clone(), String::new()))
            .collect()
    };
    assert!(TagSet::new(first(10), OBJECT_TAGS).is_ok());
    assert_eq!(
        TagSet::new(first(11), OBJECT_TAGS).err(),
        Some(TagError::TooMany)
    );
    assert!(TagSet::new(first(50), BUCKET_TAGS).is_ok());
    assert_eq!(
        TagSet::new(first(51), BUCKET_TAGS).err(),
        Some(TagError::TooMany)
    );
}

#[test]
fn the_tagging_header_is_a_percent_encoded_query() {
    let set =
        TagSet::from_header("team=blue%20sky&tier&path=a%2Fb%3Dc", OBJECT_TAGS).expect("valid");
    assert_eq!(keys(&set), ["team", "tier", "path"]);
    assert_eq!(set.tags()[0].value, "blue sky");
    assert_eq!(set.tags()[1].value, "");
    assert_eq!(set.tags()[2].value, "a/b=c");
    // `+` is a character a tag may hold, not a space.
    assert_eq!(
        TagSet::from_header("a=1+2", OBJECT_TAGS)
            .expect("valid")
            .tags()[0]
            .value,
        "1+2"
    );
    assert!(
        TagSet::from_header("", OBJECT_TAGS)
            .expect("valid")
            .tags()
            .is_empty()
    );
    assert_eq!(
        TagSet::from_header("a=%ZZ", OBJECT_TAGS).err(),
        Some(TagError::Encoding)
    );
    assert_eq!(
        TagSet::from_header("a=1&a=2", OBJECT_TAGS).err(),
        Some(TagError::DuplicateKey)
    );
}
