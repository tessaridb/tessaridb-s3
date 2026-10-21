use std::path::PathBuf;

use super::{LAYOUT, MARKER, check};
use crate::Error;

fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!(
        "tessari-s3-layout-{}",
        uuid::Uuid::new_v4().simple()
    ))
}

#[test]
fn a_directory_without_a_marker_is_marked_with_this_builds_layout() {
    for earlier in [false, true] {
        let root = scratch();
        if earlier {
            // An earlier build's drive: data files, no marker.
            std::fs::create_dir_all(root.join("ab/cd")).expect("dirs");
            std::fs::write(root.join("ab/cd/abcd"), b"TS3S").expect("a file");
        }
        check(&root).expect("served");
        let marked = std::fs::read_to_string(root.join(MARKER)).expect("marked");
        assert_eq!(marked.trim(), format!("tessaridb-s3 data layout {LAYOUT}"));
        check(&root).expect("its own marker is served");
        std::fs::remove_dir_all(&root).ok();
    }
}

#[test]
fn another_layout_stops_the_start_by_name() {
    for found in [
        format!("tessaridb-s3 data layout {}", LAYOUT.saturating_add(1)),
        "something else".to_owned(),
    ] {
        let root = scratch();
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join(MARKER), format!("{found}\n")).expect("marker");
        match check(&root) {
            Err(Error::DataLayout { found: said, reads }) => {
                assert_eq!(said, found);
                assert_eq!(reads, LAYOUT);
            }
            other => panic!("{found:?} answered {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(root.join(MARKER))
                .expect("kept")
                .trim(),
            found,
            "the marker is left as it was found"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
