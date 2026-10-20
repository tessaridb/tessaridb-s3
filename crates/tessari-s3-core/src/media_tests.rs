use super::{
    Fit, ImageFormat, Ladder, MediaError, Output, OutputError, Rule, Size, VideoFormat, matches,
    validate,
};

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
fn an_output_is_a_kind_a_size_and_a_format() {
    assert_eq!(
        Output::parse("image/1280/webp"),
        Ok(Output::Image {
            size: Size::Side(1280),
            format: ImageFormat::Webp,
            quality: None,
        })
    );
    assert_eq!(
        Output::parse("video/720p/mp4"),
        Ok(Output::Video {
            size: 720,
            format: VideoFormat::Mp4
        })
    );
    assert_eq!(
        Output::parse("poster/640/jpeg"),
        Ok(Output::Poster {
            size: Size::Side(640),
            format: ImageFormat::Jpeg,
            quality: None,
        })
    );
    assert_eq!(
        Output::parse("image/1300/webp@70"),
        Ok(Output::Image {
            size: Size::Side(1300),
            format: ImageFormat::Webp,
            quality: Some(70),
        })
    );
    assert_eq!(
        Output::parse("image/400x400-cover/avif"),
        Ok(Output::Image {
            size: Size::Box {
                width: 400,
                height: 400,
                mode: Fit::Cover
            },
            format: ImageFormat::Avif,
            quality: None,
        })
    );
    assert_eq!(
        Output::parse("poster/1280x720/jpeg@90"),
        Ok(Output::Poster {
            size: Size::Box {
                width: 1280,
                height: 720,
                mode: Fit::Fit
            },
            format: ImageFormat::Jpeg,
            quality: Some(90),
        })
    );
    for name in [
        "image/150/png",
        "image/3840/avif",
        "video/2160p/webm",
        "poster/1920/avif",
        "image/16/jpeg",
        "image/7680/webp@1",
        "image/800x600/jpeg@100",
        "image/7680x6510-exact/png",
        "image/800x600-exact/png",
        "poster/300x200-cover/webp@55",
    ] {
        assert_eq!(
            Output::parse(name).map(|output| output.name()),
            Ok(name.to_owned())
        );
    }
    assert_eq!(
        Output::parse("image/800x600-fit/jpeg").map(|output| output.name()),
        Ok("image/800x600/jpeg".to_owned()),
        "fit is the default and is not spelled"
    );
}

#[test]
fn an_output_outside_the_grammar_is_refused_by_name() {
    for (refused, why) in [
        ("image/1280/gif", OutputError::Unknown),
        ("video/720/mp4", OutputError::Unknown),
        ("video/720p/mov", OutputError::Unknown),
        ("video/700p/mp4", OutputError::Unknown),
        ("poster/720p/jpeg", OutputError::Unknown),
        ("thumb", OutputError::Unknown),
        ("image/1280", OutputError::Unknown),
        ("image/1280/webp/x", OutputError::Unknown),
        ("image/12a0/webp", OutputError::Unknown),
        ("image/800x/webp", OutputError::Unknown),
        ("image/800x600-zoom/webp", OutputError::Unknown),
        ("image/1280/webp@", OutputError::Unknown),
        ("image/1280/webp@x", OutputError::Unknown),
        ("image/15/jpeg", OutputError::SizeOutOfRange),
        ("image/7681/jpeg", OutputError::SizeOutOfRange),
        ("image/800x7681/jpeg", OutputError::SizeOutOfRange),
        ("poster/0x600/jpeg", OutputError::SizeOutOfRange),
        ("image/640-cover/jpeg", OutputError::ModeNeedsBox),
        ("image/7680x7680-exact/png", OutputError::BoxTooLarge),
        ("poster/7680x6511/jpeg", OutputError::BoxTooLarge),
        ("image/640/jpeg@0", OutputError::QualityOutOfRange),
        ("image/640/jpeg@101", OutputError::QualityOutOfRange),
        ("image/640/png@80", OutputError::QualityOnLossless),
        ("video/720p/mp4@80", OutputError::Unknown),
    ] {
        assert_eq!(Output::parse(refused), Err(why), "{refused}");
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
    assert_eq!(file("image/1300/jpeg@70"), "image-1300-q70.jpg");
    assert_eq!(file("image/400x400-cover/webp"), "image-400x400-cover.webp");
    assert_eq!(file("image/800x600/jpeg"), "image-800x600.jpg");
    assert_eq!(file("image/800x600-exact/png"), "image-800x600-exact.png");
    assert_eq!(
        file("poster/300x200-cover/avif@40"),
        "poster-300x200-cover-q40.avif"
    );
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

#[test]
fn a_stream_names_its_rungs_once_ascending_and_its_files_under_one_folder() {
    let ladder = |name: &str| Ladder::parse(name).expect("a ladder");
    assert_eq!(
        Output::parse("video/360p+720p/hls"),
        Ok(Output::Stream {
            ladder: ladder("360p+720p")
        })
    );
    assert_eq!(
        ladder("360p+720p+1080p").rungs().collect::<Vec<_>>(),
        vec![360, 720, 1080]
    );
    for name in [
        "video/720p/hls",
        "video/360p+480p+720p+1080p+1440p+2160p/hls",
    ] {
        assert_eq!(
            Output::parse(name).map(|output| output.name()),
            Ok(name.to_owned())
        );
    }
    for refused in [
        "video/720p+360p/hls",
        "video/720p+720p/hls",
        "video/700p/hls",
        "video//hls",
        "video/720p+/hls",
        "video/hls/mp4",
        "image/720p/hls",
    ] {
        assert_eq!(
            Output::parse(refused),
            Err(OutputError::Unknown),
            "{refused}"
        );
    }
    let stream = Output::parse("video/360p+720p/hls").expect("a stream");
    assert_eq!(
        stream.file_name(),
        "hls-360p-720p/master.m3u8",
        "no + to encode in a path"
    );
    assert_eq!(stream.source(), "video");
}
