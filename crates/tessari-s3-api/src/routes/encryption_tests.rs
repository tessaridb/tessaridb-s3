use tessari_s3_storage::buckets::BucketEncryption;
use tessari_s3_storage::encryption::{KeyName, Sealing};
use tessari_s3_types::ErrorCode;

use super::{document, parse};

fn configuration(rule: &str) -> Vec<u8> {
    format!(
        "<ServerSideEncryptionConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">{rule}\
         </ServerSideEncryptionConfiguration>"
    )
    .into_bytes()
}

fn by_default(inner: &str, rest: &str) -> Vec<u8> {
    configuration(&format!(
        "<Rule><ApplyServerSideEncryptionByDefault>{inner}</ApplyServerSideEncryptionByDefault>{rest}</Rule>"
    ))
}

fn named(name: &str) -> Sealing {
    Sealing::Named(KeyName::new(name).expect("usable"))
}

fn refused(document: &[u8]) -> ErrorCode {
    parse(document).expect_err("refused").code
}

#[test]
fn a_configuration_names_the_servers_key_or_a_named_one() {
    let server = parse(&by_default("<SSEAlgorithm>AES256</SSEAlgorithm>", "")).expect("reads");
    assert_eq!(
        server,
        BucketEncryption {
            sealing: Sealing::Server,
            bucket_key: false
        }
    );
    let kms = |id: &str| {
        parse(&by_default(
            &format!("<SSEAlgorithm>aws:kms</SSEAlgorithm>{id}"),
            "<BucketKeyEnabled>true</BucketKeyEnabled>",
        ))
        .expect("reads")
    };
    assert_eq!(
        kms(""),
        BucketEncryption {
            sealing: named("aws-s3"),
            bucket_key: true
        },
        "no key id names S3's own managed key"
    );
    for id in [
        "logs-key",
        "alias/logs-key",
        "arn:aws:kms:us-east-1:111122223333:key/logs-key",
    ] {
        assert_eq!(
            kms(&format!("<KMSMasterKeyID>{id}</KMSMasterKeyID>")).sealing,
            named("logs-key"),
            "{id}"
        );
    }
}

#[test]
fn a_configuration_this_server_cannot_enforce_is_refused_by_name() {
    assert_eq!(
        refused(&by_default("<SSEAlgorithm>aws:kms:dsse</SSEAlgorithm>", "")),
        ErrorCode::NotImplemented
    );
    assert_eq!(
        refused(&by_default("<SSEAlgorithm>AES128</SSEAlgorithm>", "")),
        ErrorCode::MalformedXml
    );
    assert_eq!(
        refused(&by_default(
            "<SSEAlgorithm>AES256</SSEAlgorithm><KMSMasterKeyID>logs-key</KMSMasterKeyID>",
            ""
        )),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        refused(&by_default(
            "<SSEAlgorithm>AES256</SSEAlgorithm>",
            "<BlockedEncryptionTypes><EncryptionType>NONE</EncryptionType></BlockedEncryptionTypes>"
        )),
        ErrorCode::NotImplemented,
        "customer keys cannot be allowed here"
    );
    assert_eq!(
        refused(&by_default(
            "<SSEAlgorithm>AES256</SSEAlgorithm>",
            "<BucketKeyEnabled>yes</BucketKeyEnabled>"
        )),
        ErrorCode::MalformedXml
    );
}

#[test]
fn a_configuration_of_another_shape_is_malformed() {
    let rule = "<Rule><ApplyServerSideEncryptionByDefault><SSEAlgorithm>AES256</SSEAlgorithm>\
                </ApplyServerSideEncryptionByDefault></Rule>";
    for shape in [
        configuration(""),
        configuration(&format!("{rule}{rule}")),
        configuration("<Rule><BucketKeyEnabled>true</BucketKeyEnabled></Rule>"),
        by_default("<SSEAlgorithm>AES256</SSEAlgorithm>", "<Unknown/>"),
        by_default("", ""),
        b"<VersioningConfiguration/>".to_vec(),
    ] {
        assert_eq!(
            refused(&shape),
            ErrorCode::MalformedXml,
            "{}",
            String::from_utf8_lossy(&shape)
        );
    }
}

#[test]
fn blocking_customer_keys_is_accepted_because_they_are_always_refused_here() {
    let blocked = parse(&by_default(
        "<SSEAlgorithm>AES256</SSEAlgorithm>",
        "<BlockedEncryptionTypes><EncryptionType>SSE-C</EncryptionType></BlockedEncryptionTypes>",
    ))
    .expect("reads");
    assert_eq!(blocked.sealing, Sealing::Server);
}

#[test]
fn a_configuration_reads_back_as_it_was_answered() {
    for encryption in [
        BucketEncryption {
            sealing: Sealing::Server,
            bucket_key: false,
        },
        BucketEncryption {
            sealing: named("logs-key"),
            bucket_key: true,
        },
    ] {
        let answered = document(&encryption);
        assert!(answered.contains("http://s3.amazonaws.com/doc/2006-03-01/"));
        assert_eq!(parse(answered.as_bytes()).expect("reads"), encryption);
    }
    assert!(
        document(&BucketEncryption {
            sealing: named("logs-key"),
            bucket_key: false,
        })
        .contains("<SSEAlgorithm>aws:kms</SSEAlgorithm><KMSMasterKeyID>logs-key</KMSMasterKeyID>")
    );
}
