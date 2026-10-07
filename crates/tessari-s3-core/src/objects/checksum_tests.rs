use super::{ChecksumAlgorithm, Declared, Hashes};

/// The standard check values: each algorithm over the ASCII bytes `123456789`, as the S3 header carries them —
/// base64 of the big-endian digest (CRC catalogue check values 0xCBF43926, 0xE3069283, 0xAE8B14860A799888).
#[test]
fn every_algorithm_reproduces_its_check_value() {
    let mut hashes = Hashes::new();
    hashes.update(b"12345");
    hashes.update(b"6789");
    let digests = hashes.finish();
    assert_eq!(digests.value(ChecksumAlgorithm::Crc32), "y/Q5Jg==");
    assert_eq!(digests.value(ChecksumAlgorithm::Crc32c), "4waSgw==");
    assert_eq!(digests.value(ChecksumAlgorithm::Crc64Nvme), "rosUhgp5mIg=");
    assert_eq!(
        digests.value(ChecksumAlgorithm::Sha1),
        "98O8HYCOBHMq32eZZczDTKeuNEE="
    );
    assert_eq!(
        digests.value(ChecksumAlgorithm::Sha256),
        "FeKw08M4keuw8e9gnsQZQgwg4yDOlMZfvIwzEkSOsiU="
    );
    assert_eq!(digests.md5_hex, "25f9e794323b453885f5181f1b624d0b");
}

#[test]
fn the_empty_body_has_the_published_empty_digests() {
    let digests = Hashes::new().finish();
    assert_eq!(digests.md5_hex, "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(
        digests.value(ChecksumAlgorithm::Sha256),
        "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
    );
    assert_eq!(digests.value(ChecksumAlgorithm::Crc32), "AAAAAA==");
}

#[test]
fn headers_and_names_parse_and_unknown_algorithms_are_marked_unsupported() {
    for algorithm in ChecksumAlgorithm::ALL {
        assert_eq!(
            ChecksumAlgorithm::parse(algorithm.header()),
            Some(Declared::Supported(algorithm))
        );
        assert_eq!(
            ChecksumAlgorithm::parse(algorithm.name()),
            Some(Declared::Supported(algorithm))
        );
        assert_eq!(
            ChecksumAlgorithm::parse(&algorithm.name().to_ascii_lowercase()),
            Some(Declared::Supported(algorithm))
        );
    }
    assert_eq!(
        ChecksumAlgorithm::Crc64Nvme.header(),
        "x-amz-checksum-crc64nvme"
    );
    assert_eq!(ChecksumAlgorithm::Crc64Nvme.name(), "CRC64NVME");
    for unsupported in [
        "x-amz-checksum-sha512",
        "x-amz-checksum-md5",
        "XXHASH64",
        "x-amz-checksum-xxhash3",
    ] {
        assert_eq!(
            ChecksumAlgorithm::parse(unsupported),
            Some(Declared::Unsupported),
            "{unsupported}"
        );
    }
    for other in [
        "x-amz-checksum-mode",
        "x-amz-checksum-type",
        "content-md5",
        "",
    ] {
        assert_eq!(ChecksumAlgorithm::parse(other), None, "{other}");
    }
}
