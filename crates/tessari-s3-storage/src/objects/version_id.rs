//! Where a version lives and what it is called (ADR-0010). A version's record id is the bucket, `/`, the key with
//! every `00` byte written `00 FF`, the terminator `00 01`, then the version's sequence inverted, big-endian — so ids
//! sort by key in S3 byte order and, within a key, newest first. A version id is the sequence and a random nonce in
//! hex; the null version is called `null`.

/// The id prefix every version of `key` in `bucket` shares; nothing else starts with it.
pub(crate) fn key_prefix(bucket: &str, key: &str) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(bucket.len().saturating_add(key.len()).saturating_add(3));
    prefix.extend_from_slice(bucket.as_bytes());
    prefix.push(b'/');
    for &byte in key.as_bytes() {
        prefix.push(byte);
        if byte == 0 {
            prefix.push(0xFF);
        }
    }
    prefix.extend_from_slice(&[0x00, 0x01]);
    prefix
}

/// The record id of the version numbered `seq` under `prefix`.
pub(crate) fn record_id(prefix: &[u8], seq: u64) -> Vec<u8> {
    let mut id = prefix.to_vec();
    id.extend_from_slice(&(!seq).to_be_bytes());
    id
}

/// The id of the null version.
pub(crate) const NULL: &str = "null";

/// The id of version `seq` whose nonce is `nonce`.
pub(crate) fn named(seq: u64, nonce: u64) -> String {
    format!("{seq:016x}{nonce:016x}")
}

#[cfg(test)]
mod tests {
    use super::{key_prefix, named, record_id};

    #[test]
    fn ids_sort_by_key_in_byte_order_then_newest_first() {
        let keys = ["a", "a\u{0}", "a\u{0}\u{0}", "a\u{1}", "a!", "a/x", "b"];
        let mut ids = Vec::new();
        for key in keys {
            for seq in [1, 2, 300] {
                ids.push((record_id(&key_prefix("bkt", key), seq), key, seq));
            }
        }
        ids.sort_by(|left, right| left.0.cmp(&right.0));
        let order: Vec<(&str, u64)> = ids.iter().map(|(_, key, seq)| (*key, *seq)).collect();
        let mut expected = Vec::new();
        for key in keys {
            for seq in [300, 2, 1] {
                expected.push((key, seq));
            }
        }
        assert_eq!(order, expected);
    }

    #[test]
    fn a_version_id_is_the_sequence_then_the_nonce_in_lower_hex() {
        assert_eq!(named(7, 0xdead_beef), "000000000000000700000000deadbeef");
    }
}
