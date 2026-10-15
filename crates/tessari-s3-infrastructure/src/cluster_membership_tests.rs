//! How a member takes part in membership: founding members and a join token.

use tessari_s3_types::NodeId;

use super::tests::clustered;
use crate::Error;
use crate::config::tests::{CREDENTIALS, load, with};

#[test]
fn founding_members_and_a_join_token_are_read_and_the_token_never_printed() {
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let read = |extra: &[(&'static str, &'static str)]| {
        let mut vars = clustered();
        vars.extend_from_slice(extra);
        load(&vars).map(|config| config.cluster.expect("clustered"))
    };
    let plain = read(&[]).expect("loads");
    assert!(plain.founding.is_empty() && plain.join_token.is_none());
    let joining = read(&[
        ("TESSARIDB_S3_FOUNDING_MEMBERS", " n1, n2 ,n3"),
        ("TESSARIDB_S3_JOIN_TOKEN", TOKEN),
    ])
    .expect("loads");
    let founding: Vec<&str> = joining.founding.iter().map(NodeId::as_str).collect();
    assert_eq!(founding, ["n1", "n2", "n3"]);
    assert_eq!(
        joining.join_token.as_ref().map(|token| token.expose()),
        Some(TOKEN)
    );
    assert!(
        !format!("{joining:?}").contains(TOKEN),
        "the token is never printed"
    );
    // A blank value is unset, as for every variable.
    for bad in ["n1,,n2", "N1", "n1,"] {
        assert_eq!(
            read(&[("TESSARIDB_S3_FOUNDING_MEMBERS", bad)]).map(|_| ()),
            Err(Error::InvalidConfig {
                key: "TESSARIDB_S3_FOUNDING_MEMBERS",
                reason: "node ids separated by commas",
            }),
            "{bad:?}"
        );
    }
    let mut stray = with(&CREDENTIALS);
    stray.push(("TESSARIDB_S3_FOUNDING_MEMBERS", "n1"));
    assert!(
        matches!(
            load(&stray),
            Err(Error::InvalidConfig {
                key: "TESSARIDB_S3_ERASURE",
                ..
            })
        ),
        "membership settings for a node that is not clustered are refused"
    );
}
