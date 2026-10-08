use std::collections::BTreeMap;

use super::occupancy::{Geometry, ObjectFold, Occupancy, PartFold, occupancy};

const MIB: u64 = 1 << 20;

fn fold(bucket: &str, inline_bytes: u64, data_bytes: u64, data_stripes: u64) -> ObjectFold {
    ObjectFold {
        bucket: bucket.to_owned(),
        objects: 2,
        bytes: inline_bytes.saturating_add(data_bytes),
        inline_bytes,
        data_bytes,
        data_stripes,
    }
}

const fn part(upload: u8, bytes: u64, stripes: u64) -> PartFold {
    PartFold {
        upload: [upload; 16],
        bytes,
        stripes,
    }
}

fn owners() -> BTreeMap<[u8; 16], String> {
    [([1; 16], "media".to_owned()), ([2; 16], "open".to_owned())].into()
}

#[test]
fn a_node_on_its_own_holds_its_data_files_as_they_are() {
    let found = occupancy(
        &[fold("media", 10, 3 * MIB, 3)],
        &[part(1, 6 * MIB, 6), part(9, MIB, 1)],
        &owners(),
        None,
    );
    assert_eq!(
        found,
        vec![Occupancy {
            bucket: "media".to_owned(),
            objects: 2,
            bytes: 10 + 3 * MIB,
            inline_bytes: 10,
            raw_bytes: 3 * MIB + 6 * MIB,
        }],
        "a part no upload owns is not counted"
    );
}

#[test]
fn a_cluster_counts_every_stripe_at_its_full_width() {
    // 4+2 with 1 MiB stripes: a block is 256 KiB, a stripe takes 6 blocks on the drives.
    let geometry = Geometry {
        block_len: 256 * 1024,
        width: 6,
    };
    let stripe = 6 * 256 * 1024;
    let found = occupancy(
        &[fold("media", 0, MIB + 1, 2)],
        &[part(1, 5 * MIB, 5), part(2, 1, 1)],
        &owners(),
        Some(geometry),
    );
    assert_eq!(
        found,
        vec![
            Occupancy {
                bucket: "media".to_owned(),
                objects: 2,
                bytes: MIB + 1,
                inline_bytes: 0,
                raw_bytes: 7 * stripe,
            },
            Occupancy {
                bucket: "open".to_owned(),
                objects: 0,
                bytes: 0,
                inline_bytes: 0,
                raw_bytes: stripe,
            },
        ],
        "an open upload's one byte still takes a whole stripe"
    );
}
