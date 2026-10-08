use super::{Code, CodeError};

#[test]
fn a_code_is_k_data_and_m_parity_shards_with_their_quorums() {
    let code = Code::new(4, 2).expect("4+2 is a code");
    assert_eq!((code.data(), code.parity(), code.width()), (4, 2, 6));
    assert_eq!(code.read_quorum(), 4, "any k shards read");
    assert_eq!(code.write_quorum(), 4, "k durable shards when k > m");
}

#[test]
fn half_parity_needs_one_more_shard_to_write() {
    for (data, parity, quorum) in [(2, 2, 3), (1, 1, 2), (8, 8, 9), (3, 2, 3), (2, 3, 2)] {
        let code = Code::new(data, parity).expect("a code");
        assert_eq!(code.write_quorum(), quorum, "{data}+{parity}");
    }
}

#[test]
fn a_code_without_data_or_parity_or_wider_than_sixteen_is_refused() {
    for (data, parity) in [(0, 2), (4, 0), (0, 0), (10, 7), (16, 1)] {
        assert_eq!(
            Code::new(data, parity),
            Err(CodeError::Shape { data, parity }),
            "{data}+{parity}"
        );
    }
    assert!(Code::new(15, 1).is_ok(), "sixteen shards is the widest");
}

#[test]
fn a_code_is_written_as_k_plus_m() {
    assert_eq!(Code::parse("4+2"), Code::new(4, 2));
    assert_eq!(Code::parse("12+4"), Code::new(12, 4));
    for text in [
        "4", "4+", "+2", "a+b", "4+2+1", " 4+2", "4 + 2", "-1+2", "300+2",
    ] {
        assert_eq!(Code::parse(text), Err(CodeError::Syntax), "{text:?}");
    }
    assert_eq!(
        Code::parse("4+0"),
        Err(CodeError::Shape { data: 4, parity: 0 })
    );
}
