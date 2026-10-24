use std::time::Duration;

use super::LiveSettings;
use crate::config::tests::{load, with};

#[test]
fn a_republished_configuration_changes_the_live_values_and_names_what_changed() {
    let first = load(&with(&[
        ("TESSARIDB_S3_RECLAIM_INTERVAL_SECS", "300"),
        ("TESSARIDB_S3_USAGE_INTERVAL_SECS", "60"),
    ]))
    .expect("loads");
    let live = LiveSettings::new(&first);
    let reclaim = live.reclaim_interval();
    assert_eq!(reclaim.secs(), Duration::from_secs(300));
    assert!(live.domains().is_empty());
    // The same values again change nothing — the control for the list below.
    assert!(live.publish(&first).is_empty());
    let second = load(&with(&[
        ("TESSARIDB_S3_RECLAIM_INTERVAL_SECS", "30"),
        ("TESSARIDB_S3_USAGE_INTERVAL_SECS", "60"),
        ("TESSARIDB_S3_DOMAINS", "s3.example.test"),
    ]))
    .expect("loads");
    assert_eq!(
        live.publish(&second),
        ["TESSARIDB_S3_RECLAIM_INTERVAL_SECS", "TESSARIDB_S3_DOMAINS"]
    );
    // A cell handed out before the change reads the new value: the daemons hold cells, not numbers.
    assert_eq!(reclaim.secs(), Duration::from_secs(30));
    assert_eq!(live.usage_interval().get(), 60);
    assert_eq!(*live.domains(), ["s3.example.test"]);
}
