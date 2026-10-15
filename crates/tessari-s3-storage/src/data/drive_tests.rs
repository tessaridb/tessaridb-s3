use super::super::files::DataFiles;
use super::DriveSpace;

#[tokio::test]
async fn the_drive_reports_a_size_with_free_space_inside_it() {
    let files = DataFiles::new(std::env::temp_dir(), 64);
    let drive = files.drive().await.expect("the temp dir's filesystem");
    assert!(drive.capacity > 0, "{drive:?}");
    assert!(drive.free <= drive.capacity, "{drive:?}");
    assert!(drive.available <= drive.free, "{drive:?}");
}

#[tokio::test]
async fn a_data_directory_not_yet_created_reports_the_filesystem_it_will_be_on() {
    let missing =
        std::env::temp_dir().join(format!("tessari-s3-none-{}", uuid::Uuid::new_v4().simple()));
    let parent = DataFiles::new(std::env::temp_dir(), 64)
        .drive()
        .await
        .expect("temp dir");
    let drive = DataFiles::new(missing, 64)
        .drive()
        .await
        .expect("its parent's filesystem");
    assert_eq!(drive.capacity, parent.capacity);
}

#[test]
fn the_wire_form_reads_back_and_refuses_anything_else() {
    let drive = DriveSpace {
        capacity: 500,
        free: 200,
        available: 150,
    };
    assert_eq!(drive.to_wire(), "500 200 150");
    assert_eq!(DriveSpace::from_wire(&drive.to_wire()), Some(drive));
    for bad in ["", "1 2", "1 2 3 4", "1 2 x", "-1 2 3", "1  2 3"] {
        assert_eq!(DriveSpace::from_wire(bad), None, "{bad:?}");
    }
}
