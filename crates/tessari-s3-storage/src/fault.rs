//! Exit points a process test arms to stop the node at a named place inside a commit, as a power cut would, and then
//! check what the restarted node shows. They exist only in a build with the `fault-points` feature, which only the
//! process tests turn on; everywhere else they compile to nothing.

/// Aborts the process when `TESSARIDB_S3_TEST_EXIT_AT` names `point`.
#[cfg(feature = "fault-points")]
pub(crate) fn exit_at(point: &str) {
    if std::env::var("TESSARIDB_S3_TEST_EXIT_AT").is_ok_and(|armed| armed == point) {
        tracing::error!(point, "stopping at an armed exit point");
        std::process::abort();
    }
}

/// Nothing: this build has no exit points.
#[cfg(not(feature = "fault-points"))]
pub(crate) const fn exit_at(_point: &str) {}
