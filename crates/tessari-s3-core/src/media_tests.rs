use super::{MediaError, Preset, Rule, matches, validate};

fn rule(id: &str, types: &[&str], prefix: &str, outputs: &[Preset]) -> Rule {
    Rule {
        id: id.to_owned(),
        content_types: types.iter().map(|kind| (*kind).to_owned()).collect(),
        prefix: prefix.to_owned(),
        outputs: outputs.to_vec(),
    }
}

#[test]
fn a_preset_is_named_and_knows_its_kind() {
    assert_eq!(Preset::parse("thumb"), Some(Preset::Thumb));
    assert_eq!(Preset::parse("720p"), Some(Preset::Video720));
    assert_eq!(Preset::parse("gif"), None);
    assert_eq!(Preset::Thumb.name(), "thumb");
    assert!(Preset::Web.is_image() && !Preset::Poster.is_image());
}

#[test]
fn rules_are_checked_before_they_are_stored() {
    let photos = rule(
        "photos",
        &["image/*"],
        "photos/",
        &[Preset::Thumb, Preset::Web],
    );
    let clips = rule(
        "clips",
        &["video/mp4"],
        "",
        &[Preset::Video720, Preset::Poster],
    );
    assert_eq!(validate(&[photos.clone(), clips]), Ok(()));
    assert_eq!(validate(&[]), Ok(()), "no rules turns media processing off");
    assert_eq!(
        validate(&[rule("a", &[], "", &[Preset::Thumb])]),
        Err(MediaError::NoContentType)
    );
    assert_eq!(
        validate(&[rule("a", &["image/*"], "", &[])]),
        Err(MediaError::NoOutput)
    );
    assert_eq!(
        validate(&[rule("a", &["image"], "", &[Preset::Thumb])]),
        Err(MediaError::BadContentType("image".to_owned()))
    );
    assert_eq!(
        validate(&[rule("a", &["image/png"], "", &[Preset::Video720])]),
        Err(MediaError::WrongKind("720p"))
    );
    assert_eq!(
        validate(&[rule("a", &["video/*"], "", &[Preset::Thumb])]),
        Err(MediaError::WrongKind("thumb"))
    );
    assert_eq!(
        validate(&[photos.clone(), photos]),
        Err(MediaError::DuplicateId)
    );
    assert_eq!(
        validate(&[rule("a", &["image/*"], "x.renditions/", &[Preset::Thumb])]),
        Err(MediaError::RenditionPrefix)
    );
}

#[test]
fn a_rule_matches_its_types_and_prefix_and_never_a_rendition() {
    let photos = rule(
        "photos",
        &["image/*", "application/pdf"],
        "photos/",
        &[Preset::Thumb],
    );
    assert!(matches(&photos, "image/jpeg", "photos/a.jpg"));
    assert!(matches(
        &photos,
        "IMAGE/PNG; charset=binary",
        "photos/b.png"
    ));
    assert!(matches(&photos, "application/pdf", "photos/c.pdf"));
    // Control arms: another type, another prefix, and the rule's own outputs.
    assert!(!matches(&photos, "video/mp4", "photos/a.mp4"));
    assert!(!matches(&photos, "image/jpeg", "docs/a.jpg"));
    assert!(!matches(
        &photos,
        "image/webp",
        "photos/a.jpg.renditions/thumb.webp"
    ));
}
