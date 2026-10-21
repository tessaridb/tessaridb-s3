use std::ffi::OsString;
use std::path::Path;

use tessari_s3_constants::MEDIA_FFMPEG_INPUT_FORMATS;

use super::command_line;

/// The value given to `option` before the input is named, if any.
fn input_option(line: &[OsString], option: &str) -> Option<String> {
    let input = line.iter().position(|arg| arg == "-i")?;
    line[..input]
        .windows(2)
        .find(|pair| pair[0] == option)
        .map(|pair| pair[1].to_string_lossy().into_owned())
}

#[test]
fn an_original_is_read_only_as_a_local_file_in_a_listed_container() {
    // An uploaded original is hostile: as a playlist or a concat list it would make ffmpeg open other local files or
    // fetch from the network, so the input may use only the file protocol and only the listed demuxers.
    let line = command_line(Path::new("/tmp/x.in"), &["-f", "null"], Path::new("-"));
    assert_eq!(
        input_option(&line, "-protocol_whitelist").as_deref(),
        Some("file")
    );
    assert_eq!(
        input_option(&line, "-format_whitelist").as_deref(),
        Some(MEDIA_FFMPEG_INPUT_FORMATS)
    );
    for refused in ["hls", "concat", "image2", "tee"] {
        assert!(
            !MEDIA_FFMPEG_INPUT_FORMATS
                .split(',')
                .any(|name| name == refused),
            "{refused} is no container an original arrives in"
        );
    }
    assert_eq!(line.last().map(OsString::as_os_str), Some("-".as_ref()));
}
