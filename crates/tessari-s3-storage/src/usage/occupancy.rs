//! Real occupancy: from what one usage pass read, each bucket's bytes held in the metadata store (inline objects)
//! and on the drives (data files, with erasure overhead on a cluster). Pure: everything it reads is in its arguments.

use std::collections::BTreeMap;

/// One bucket's objects as the pass folded them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObjectFold {
    pub(crate) bucket: String,
    pub(crate) objects: u64,
    /// Logical bytes of every object.
    pub(crate) bytes: u64,
    /// Bytes of the objects held inline, in the metadata.
    pub(crate) inline_bytes: u64,
    /// Bytes of the single-part objects held in data files.
    pub(crate) data_bytes: u64,
    /// Stripes those data files are cut into, summed over files.
    pub(crate) data_stripes: u64,
}

/// One upload's parts — of an open upload, or of a completed multipart object — as the pass folded them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartFold {
    pub(crate) upload: [u8; 16],
    pub(crate) bytes: u64,
    pub(crate) stripes: u64,
}

/// Everything one pass reads: the objects folded per bucket, the parts folded per upload, and which bucket each
/// upload — open or completed — belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Measured {
    pub(crate) objects: Vec<ObjectFold>,
    pub(crate) parts: Vec<PartFold>,
    pub(crate) owners: BTreeMap<[u8; 16], String>,
}

/// How a cluster lays a stripe out on its drives; a node on its own stores a data file as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Geometry {
    /// Bytes of one block (one shard's share of a stripe).
    pub(crate) block_len: u64,
    /// Blocks per stripe: k + m.
    pub(crate) width: u64,
}

/// One bucket's figures, ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Occupancy {
    pub(crate) bucket: String,
    pub(crate) objects: u64,
    pub(crate) bytes: u64,
    pub(crate) inline_bytes: u64,
    pub(crate) raw_bytes: u64,
}

/// Each bucket's occupancy: parts count toward the bucket `owners` names for their upload (a part no upload owns
/// is left out — it is garbage the reclaimer removes). A bucket holding only open-upload parts appears with no
/// objects.
pub(crate) fn occupancy(
    objects: &[ObjectFold],
    parts: &[PartFold],
    owners: &BTreeMap<[u8; 16], String>,
    geometry: Option<Geometry>,
) -> Vec<Occupancy> {
    // The bytes `stripes` stripes of `bytes` logical bytes take on the drives.
    let raw = |bytes: u64, stripes: u64| match geometry {
        Some(layout) => stripes
            .saturating_mul(layout.block_len)
            .saturating_mul(layout.width),
        None => bytes,
    };
    let mut buckets: BTreeMap<&str, Occupancy> = BTreeMap::new();
    for fold in objects {
        buckets.insert(
            &fold.bucket,
            Occupancy {
                bucket: fold.bucket.clone(),
                objects: fold.objects,
                bytes: fold.bytes,
                inline_bytes: fold.inline_bytes,
                raw_bytes: raw(fold.data_bytes, fold.data_stripes),
            },
        );
    }
    for part in parts {
        let Some(bucket) = owners.get(&part.upload) else {
            continue;
        };
        let entry = buckets.entry(bucket).or_insert_with(|| Occupancy {
            bucket: bucket.clone(),
            objects: 0,
            bytes: 0,
            inline_bytes: 0,
            raw_bytes: 0,
        });
        entry.raw_bytes = entry
            .raw_bytes
            .saturating_add(raw(part.bytes, part.stripes));
    }
    buckets.into_values().collect()
}
