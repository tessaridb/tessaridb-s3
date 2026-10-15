use super::{StripeError, Stripes};
use crate::erasure::Code;

const MIB: u32 = 1024 * 1024;

fn stripes(data: u8, parity: u8, size: u32) -> Stripes {
    Stripes::new(Code::new(data, parity).expect("a code"), size).expect("a stripe size")
}

/// Deterministic bytes that are not a pattern a codec could reproduce by accident.
fn bytes(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()[0]
        })
        .collect()
}

fn present(blocks: &[Vec<u8>]) -> Vec<Option<Vec<u8>>> {
    blocks.iter().cloned().map(Some).collect()
}

#[test]
fn a_stripe_is_cut_into_k_blocks_of_a_kth_rounded_up() {
    assert_eq!(stripes(4, 2, MIB).block_len(), 262_144);
    assert_eq!(stripes(3, 2, 10).block_len(), 4, "10 / 3 rounded up");
    assert_eq!(
        stripes(3, 2, 13).block_len(),
        6,
        "13 / 3 rounded up to an even count"
    );
    let set = stripes(4, 2, MIB);
    for (size, count) in [(0, 0), (1, 1), (u64::from(MIB), 1), (u64::from(MIB) + 1, 2)] {
        assert_eq!(set.stripe_count(size), count, "{size} bytes");
    }
    assert_eq!(
        Stripes::new(Code::new(4, 2).expect("a code"), 0).err(),
        Some(StripeError::Size)
    );
}

#[test]
fn any_k_of_the_k_plus_m_blocks_give_the_stripe_back() {
    let set = stripes(4, 2, 1024);
    let stripe = bytes(1000, 7);
    let blocks = set.encode(&stripe).expect("encoded");
    assert_eq!(blocks.len(), 6);
    assert!(blocks.iter().all(|block| block.len() == 256));
    for lost_a in 0..6 {
        for lost_b in lost_a..6 {
            let mut held = present(&blocks);
            held[lost_a] = None;
            held[lost_b] = None;
            assert_eq!(
                set.decode(&mut held, stripe.len()),
                Ok(stripe.clone()),
                "blocks {lost_a} and {lost_b} lost"
            );
        }
    }
}

#[test]
fn decoding_reads_the_blocks_it_is_given() {
    let set = stripes(4, 2, 1024);
    let stripe = bytes(1024, 11);
    let blocks = set.encode(&stripe).expect("encoded");
    let mut damaged = present(&blocks);
    damaged[1] = Some(vec![0; 256]);
    assert_ne!(
        set.decode(&mut damaged, stripe.len()),
        Ok(stripe.clone()),
        "a wrong block present is believed — verifying it is the caller's job"
    );
    assert!(
        blocks[4..]
            .iter()
            .any(|parity| parity.iter().any(|b| *b != 0)),
        "parity carries information"
    );
}

#[test]
fn fewer_than_k_blocks_is_named() {
    let set = stripes(4, 2, 1024);
    let blocks = set.encode(&bytes(1024, 3)).expect("encoded");
    let mut held = present(&blocks);
    held[0] = None;
    held[3] = None;
    held[5] = None;
    assert_eq!(
        set.decode(&mut held, 1024),
        Err(StripeError::TooFewShards { have: 3, need: 4 })
    );
}

#[test]
fn a_stripe_of_the_wrong_size_or_a_wrong_block_set_is_refused() {
    let set = stripes(4, 2, 1024);
    assert_eq!(set.encode(&[]), Err(StripeError::Size));
    assert_eq!(set.encode(&bytes(1025, 1)), Err(StripeError::Size));
    let blocks = set.encode(&bytes(1024, 5)).expect("encoded");
    let mut five = present(&blocks[..5]);
    assert_eq!(set.decode(&mut five, 1024), Err(StripeError::Shape));
    let mut short = present(&blocks);
    short[2] = Some(vec![0; 255]);
    assert_eq!(set.decode(&mut short, 1024), Err(StripeError::Shape));
    let mut held = present(&blocks);
    assert_eq!(set.decode(&mut held, 1025), Err(StripeError::Size));
}

#[test]
fn the_widest_code_survives_losing_all_its_parity_worth() {
    let set = stripes(12, 4, MIB);
    let stripe = bytes(usize::try_from(MIB).expect("fits"), 13);
    let blocks = set.encode(&stripe).expect("encoded");
    let mut held = present(&blocks);
    for lost in [0, 5, 11, 14] {
        held[lost] = None;
    }
    assert_eq!(set.decode(&mut held, stripe.len()), Ok(stripe));
}
