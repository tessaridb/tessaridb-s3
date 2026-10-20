//! The data directory's layout: `s3data/LAYOUT` names the version of the directory — how paths are sharded and which
//! data file formats it may hold. A node checks it at start before it serves: a directory without one (a fresh drive,
//! or one an earlier build wrote, whose files every build since reads) is marked with this build's layout, the same
//! layout is served, and any other answer stops the start by name, so a build never reads files it does not know.

use std::io::{ErrorKind, Write};
use std::path::Path;

use super::files::{io, sync_dir};
use crate::{Error, Result};

/// The layout this build writes and reads: paths `xx/yy/<id>`, data file formats 1 to 4.
pub(crate) const LAYOUT: u32 = 1;

/// The marker's name under the data root.
pub(crate) const MARKER: &str = "LAYOUT";

/// What the marker of [`LAYOUT`] says.
fn marker_text() -> String {
    format!("tessaridb-s3 data layout {LAYOUT}")
}

/// Checks the directory `data_root` against [`LAYOUT`], marking it when it carries no marker; blocking.
pub(crate) fn check(data_root: &Path) -> Result<()> {
    let marker = data_root.join(MARKER);
    match std::fs::read_to_string(&marker) {
        Ok(found) if found.trim() == marker_text() => Ok(()),
        Ok(found) => Err(Error::DataLayout {
            found: found.trim().to_owned(),
            reads: LAYOUT,
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => mark(data_root),
        Err(error) => Err(io("read layout")(error)),
    }
}

/// Writes this build's marker whole or not at all: a temporary name, synced, renamed, the directory synced.
fn mark(data_root: &Path) -> Result<()> {
    std::fs::create_dir_all(data_root).map_err(io("create directory"))?;
    let temp = data_root.join(format!("{MARKER}.tmp"));
    let mut file = std::fs::File::create(&temp).map_err(io("create layout"))?;
    file.write_all(format!("{}\n", marker_text()).as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(io("write layout"))?;
    std::fs::rename(&temp, data_root.join(MARKER)).map_err(io("rename layout"))?;
    sync_dir(data_root)
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
