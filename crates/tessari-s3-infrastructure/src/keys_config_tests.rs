use super::keys_passphrase;

#[test]
fn the_keys_passphrase_is_read_when_set_and_never_shown() {
    assert!(keys_passphrase(&|_: &str| None).is_none());
    assert!(keys_passphrase(&|_: &str| Some(String::new())).is_none());
    let read = keys_passphrase(&|key: &str| {
        (key == "TESSARIDB_S3_KEYS_PASSPHRASE").then(|| "correct horse battery".to_owned())
    })
    .expect("set");
    assert_eq!(read.expose(), "correct horse battery");
    assert!(!format!("{read:?}").contains("horse"));
}
