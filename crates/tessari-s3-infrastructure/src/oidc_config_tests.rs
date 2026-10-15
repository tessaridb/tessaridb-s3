use crate::Error;
use crate::config::tests::load;
use crate::console_config::tests::{OIDC, with};

#[test]
fn key_sign_in_to_the_console_is_all_keys_unless_set_to_root_beside_a_provider() {
    use crate::console_config::KeySignIn;
    let provider = |extra: Option<&'static str>| {
        let mut vars = with(&OIDC);
        if let Some(value) = extra {
            vars.push(("TESSARIDB_S3_CONSOLE_KEY_SIGN_IN", value));
        }
        load(&vars).map(|config| {
            config
                .console
                .expect("console")
                .oidc
                .expect("provider")
                .key_sign_in
        })
    };
    assert_eq!(provider(None).expect("loads"), KeySignIn::All);
    assert_eq!(provider(Some("all")).expect("loads"), KeySignIn::All);
    assert_eq!(provider(Some(" root ")).expect("loads"), KeySignIn::Root);
    assert!(matches!(
        provider(Some("none")),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_CONSOLE_KEY_SIGN_IN",
            reason: "neither all nor root",
        })
    ));
    // Without a provider nobody could sign in but root, and root without a provider is not an emergency path.
    let mut alone = with(&[("TESSARIDB_S3_CONSOLE_LISTEN", "127.0.0.1:9101")]);
    alone.push(("TESSARIDB_S3_CONSOLE_KEY_SIGN_IN", "root"));
    assert!(matches!(
        load(&alone).map(|_| ()),
        Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_CONSOLE_KEY_SIGN_IN",
            reason: "set without TESSARIDB_S3_CONSOLE_OIDC_ISSUER",
        })
    ));
}
