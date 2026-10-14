use super::{ImageFormat, MediaError, Output, Rule, VideoFormat, matches, validate};

fn rule(id: &str, types: &[&str], prefix: &str, outputs: &[&str]) -> Rule {
    Rule {
        id: id.to_owned(),
        content_types: types.iter().map(|kind| (*kind).to_owned()).collect(),
        prefix: prefix.to_owned(),
        outputs: outputs
            .iter()
            .map(|name| Output::parse(name).expect("an output"))
            .collect(),
    }
}

#[test]
fn an_output_is_a_kind_a_popular_size_and_a_format() {
    assert_eq!(
        Output::parse("image/1280/webp"),
        Some(Output::Image {
            size: 1280,
            format: ImageFormat::Webp
        })
    );
    assert_eq!(
        Output::parse("video/720p/mp4"),
        Some(Output::Video {
            size: 720,
            format: VideoFormat::Mp4
        })
    );
    assert_eq!(
        Output::parse("poster/640/jpeg"),
        Some(Output::Poster {
            size: 640,
            format: ImageFormat::Jpeg
        })
    );
    for refused in [
        "image/1300/webp",
        "image/1280/gif",
        "video/720/mp4",
        "video/720p/mov",
        "poster/720p/jpeg",
        "thumb",
        "image/1280",
        "image/1280/webp/x",
    ] {
        assert_eq!(Output::parse(refused), None, "{refused}");
    }
    for name in [
        "image/150/png",
        "image/3840/avif",
        "video/2160p/webm",
        "poster/1920/avif",
    ] {
        assert_eq!(
            Output::parse(name).map(|output| output.name()),
            Some(name.to_owned())
        );
    }
}

#[test]
fn an_output_names_its_rendition_file() {
    let file = |name: &str| Output::parse(name).expect("an output").file_name();
    assert_eq!(file("image/320/jpeg"), "image-320.jpg");
    assert_eq!(file("image/1280/webp"), "image-1280.webp");
    assert_eq!(file("video/1080p/mp4"), "video-1080p.mp4");
    assert_eq!(file("video/480p/webm"), "video-480p.webm");
    assert_eq!(file("poster/640/avif"), "poster-640.avif");
}

#[test]
fn rules_are_checked_before_they_are_stored() {
    assert_eq!(validate(&[]), Ok(()));
    let both = [
        rule(
            "photos",
            &["image/*"],
            "photos/",
            &["image/320/jpeg", "image/1920/webp"],
        ),
        rule(
            "clips",
            &["video/mp4", "video/*"],
            "",
            &["video/720p/mp4", "poster/640/jpeg"],
        ),
    ];
    assert_eq!(validate(&both), Ok(()));
    assert_eq!(
        validate(&[rule("a", &[], "", &["image/320/jpeg"])]),
        Err(MediaError::NoContentType)
    );
    assert_eq!(
        validate(&[rule("a", &["image/*"], "", &[])]),
        Err(MediaError::NoOutput)
    );
    assert_eq!(
        validate(&[rule("a", &["image"], "", &["image/320/jpeg"])]),
        Err(MediaError::BadContentType("image".to_owned()))
    );
    assert_eq!(
        validate(&[rule("a", &["image/png"], "", &["video/720p/mp4"])]),
        Err(MediaError::WrongKind("video/720p/mp4".to_owned()))
    );
    assert_eq!(
        validate(&[rule("a", &["video/*"], "", &["image/320/jpeg"])]),
        Err(MediaError::WrongKind("image/320/jpeg".to_owned()))
    );
    assert_eq!(
        validate(&[rule("a", &["image/*"], "", &["poster/320/jpeg"])]),
        Err(MediaError::WrongKind("poster/320/jpeg".to_owned()))
    );
    assert_eq!(
        validate(&[rule(
            "a",
            &["image/*"],
            "",
            &["image/320/jpeg", "image/320/jpeg"]
        )]),
        Err(MediaError::RepeatedOutput)
    );
    assert_eq!(
        validate(&[
            rule("a", &["image/*"], "", &["image/320/jpeg"]),
            rule("a", &["image/*"], "", &["image/320/jpeg"])
        ]),
        Err(MediaError::DuplicateId)
    );
    assert_eq!(
        validate(&[rule(
            "a",
            &["image/*"],
            "x.renditions/",
            &["image/320/jpeg"]
        )]),
        Err(MediaError::RenditionPrefix)
    );
}

#[test]
fn a_rule_matches_its_types_and_prefix_and_never_a_rendition() {
    let photos = rule(
        "photos",
        &["image/*", "application/pdf"],
        "photos/",
        &["image/320/jpeg"],
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
        "photos/a.jpg.renditions/image-320.webp"
    ));
}
