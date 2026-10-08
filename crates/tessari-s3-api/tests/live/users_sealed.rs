//! How an access key's secret is stored, against a real node: the record holds it sealed, and the same record copied
//! under another key id does not open.

use tessari_s3_infrastructure::tessaridb::{Answer, Value};
use tessari_s3_storage::Error;

use crate::IGNORED;
use crate::users::{user, with_ann};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_stored_secret_is_sealed_and_does_not_open_under_another_key_id() {
    let _why = IGNORED;
    let (storage, planter) = with_ann().await;
    let issued = storage
        .users()
        .issue_key(&user("ann"))
        .await
        .expect("issued")
        .expect("ann");
    let id = vec![("id".to_owned(), Value::String(issued.access_key_id.clone()))];
    let answers = planter
        .run("SELECT * FROM ONLY access_keys:$id;", id)
        .await
        .expect("read");
    let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
        panic!("one record");
    };
    let Some((_, Value::Object(fields))) = records.into_iter().next() else {
        panic!("the key's record");
    };
    let Some(Value::Bytes(sealed)) = fields.get("secret") else {
        panic!("sealed bytes");
    };
    let plain = issued.secret.expose().as_bytes();
    assert!(
        !sealed
            .windows(8)
            .any(|window| plain.windows(8).any(|part| part == window)),
        "no 8 bytes of the secret are stored"
    );

    // The same record under another id: every stored byte is genuine, only its place changed.
    let mut copied = vec![(
        "id".to_owned(),
        Value::String("TSCOPIED000000000000".to_owned()),
    )];
    copied.extend(fields);
    planter
        .run(
            "CREATE access_keys:$id = { user_name: $user_name, secret: $secret, nonce: $nonce, kek_id: $kek_id, \
             algorithm: $algorithm, disabled: $disabled, created: $created };",
            copied,
        )
        .await
        .expect("copied");
    assert_eq!(
        storage.users().resolve("TSCOPIED000000000000").await.err(),
        Some(Error::Unsealable)
    );
}
