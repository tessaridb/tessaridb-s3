//! Encryption-key custody against a real node: the first unseal creates the keys vault and its first key, a second
//! node opening it at the same moment holds the SAME key, a wrong passphrase leaves a node sealed, and the vault is
//! sealed again on the engine once the keys are read.

use tessari_s3_infrastructure::tessaridb::{MetaError, MetaPool, SealState, Value};
use tessari_s3_storage::Error;
use tessari_s3_storage::encryption::{DataKey, open_block, seal_block, unwrap, wrap};
use tessari_s3_storage::keys::{KeyState, Passphrase};
use zeroize::Zeroizing;

use crate::{IGNORED, confined, live_config, prepared, scratch_dir, test_node};

fn passphrase(text: &str) -> Passphrase {
    Passphrase::new(Zeroizing::new(text.to_owned())).expect("a usable passphrase")
}

const RIGHT: &str = "a keys passphrase for the live suite";

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn two_nodes_unsealing_a_new_store_at_once_hold_one_key_and_a_wrong_passphrase_holds_none() {
    let _why = IGNORED;
    let config = live_config(test_node(), None, &scratch_dir());
    let one = prepared(&config).await;
    let two = prepared(&config).await;
    let three = prepared(&config).await;
    assert_eq!(
        one.keys().state().await.expect("asked"),
        KeyState::Uninitialised
    );
    assert!(matches!(one.keys().ring(), Err(Error::KeysSealed)));

    let (right_one, right_two) = (passphrase(RIGHT), passphrase(RIGHT));
    let (first, second) =
        tokio::join!(one.keys().unseal(&right_one), two.keys().unseal(&right_two));
    let unsealed = KeyState::Unsealed {
        active: "server_1".to_owned(),
        keys: 1,
    };
    assert_eq!(first.expect("one unsealed"), unsealed);
    assert_eq!(second.expect("two unsealed"), unsealed);

    // One key, not two that happen to share a name: what one node seals the other opens.
    let ring_one = one.keys().ring().expect("held");
    let ring_two = two.keys().ring().expect("held");
    let data = [3; 16];
    let key = DataKey::generate().expect("a data key");
    let wrapped = wrap(ring_one.active(), &key, data).expect("wrapped");
    let opened = unwrap(
        ring_two.get(&wrapped.kek_id).expect("the same key id"),
        &wrapped,
        data,
    )
    .expect("node two unwraps node one's data key");
    let block = seal_block(&key, data, 0, true, b"plain").expect("sealed");
    assert_eq!(
        open_block(&opened, data, 0, true, &block).expect("opens"),
        b"plain"
    );

    // The engine is sealed again once the keys are read.
    let account = MetaPool::new(confined(&config).await).expect("pool");
    let mut link = account.vault_link("s3_keys").await.expect("a link");
    assert_eq!(
        link.status().await.expect("status").state,
        SealState::Sealed
    );

    // A wrong passphrase: refused by name, and that node holds nothing.
    let wrong = three
        .keys()
        .unseal(&passphrase("not the keys passphrase at all"))
        .await;
    assert!(matches!(wrong, Err(Error::WrongPassphrase)), "{wrong:?}");
    assert!(matches!(three.keys().ring(), Err(Error::KeysSealed)));
    assert_eq!(three.keys().state().await.expect("asked"), KeyState::Sealed);

    // An existing key is never overwritten: the create a racing node sends is refused, and the key stays the one
    // node one wrapped under.
    let overwrite = link
        .run(
            "CREATE s3_keys:$id = { key: $key };",
            vec![
                ("id".to_owned(), Value::String("server_1".to_owned())),
                ("key".to_owned(), Value::Bytes(vec![9; 32])),
            ],
        )
        .await;
    assert!(
        matches!(overwrite, Err(MetaError::Refused { .. })),
        "{overwrite:?}"
    );
    three
        .keys()
        .unseal(&passphrase(RIGHT))
        .await
        .expect("three unseals");
    let ring_three = three.keys().ring().expect("held");
    unwrap(
        ring_three.get(&wrapped.kek_id).expect("the same key id"),
        &wrapped,
        data,
    )
    .expect("the key is still the one node one wrapped under");

    // Sealing a node forgets its keys; the others keep theirs.
    one.keys().seal();
    assert!(matches!(one.keys().ring(), Err(Error::KeysSealed)));
    assert!(two.keys().ring().is_ok());
}
