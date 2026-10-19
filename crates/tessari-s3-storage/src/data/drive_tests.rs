use super::super::files::DataFiles;
use super::DriveSpace;
use crate::Error;

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

#[tokio::test]
async fn below_its_free_space_floor_the_drive_takes_no_new_file_and_still_reads() {
    let root = std::env::temp_dir().join(format!(
        "tessari-s3-floor-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let files = DataFiles::new(root.clone(), 64);
    let mut kept = files.create([1; 16]).await.expect("no floor yet");
    kept.append(b"kept").await.expect("append");
    kept.finish().await.expect("finish");

    files.set_floor(u64::MAX);
    let refused = files.create([2; 16]).await.map(|_| ());
    assert!(
        matches!(
            refused,
            Err(Error::DriveFull {
                floor: u64::MAX,
                ..
            })
        ),
        "{refused:?}"
    );
    let shard = files.create_shard([3; 16], 0, 64, false).await.map(|_| ());
    assert!(matches!(shard, Err(Error::DriveFull { .. })), "{shard:?}");
    assert!(
        !files.path([2; 16]).with_extension("tmp").exists(),
        "nothing started"
    );
    let reader = files.open([1; 16], 4).await.expect("reads are served");
    assert_eq!(reader.read_block(0).await.expect("block"), b"kept");

    files.set_floor(0);
    files
        .create([4; 16])
        .await
        .expect("a floor of 0 takes every write");
    std::fs::remove_dir_all(&root).expect("cleanup");
}
