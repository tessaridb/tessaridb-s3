use super::{CUSTOMER_KEK_ID, CustomerKey, CustomerSeal};
use crate::encryption::{DataKey, unwrap, wrap};

fn key(byte: u8) -> CustomerKey {
    CustomerKey::new([byte; 32])
}

#[test]
fn a_seal_recognises_the_key_it_was_made_for_and_no_other() {
    let seal = CustomerSeal::new(&key(7)).expect("sealed");
    assert!(seal.matches(&key(7)).expect("derived"));
    assert!(!seal.matches(&key(8)).expect("derived"));
    let mut changed = seal.clone();
    changed.verifier[0] ^= 1;
    assert!(
        !changed.matches(&key(7)).expect("derived"),
        "a changed verifier recognises nothing"
    );
}

#[test]
fn each_seal_draws_its_own_salt_so_the_same_key_wraps_differently() {
    let (one, two) = (
        CustomerSeal::new(&key(7)).expect("one"),
        CustomerSeal::new(&key(7)).expect("two"),
    );
    assert_ne!(one.salt, two.salt);
    assert_ne!(one.verifier, two.verifier);
    let data = [3_u8; 16];
    let wrapped = wrap(
        &one.kek(&key(7)).expect("derived"),
        &DataKey::generate().expect("key"),
        data,
    )
    .expect("wrapped");
    assert_eq!(wrapped.kek_id, CUSTOMER_KEK_ID);
    assert!(unwrap(&one.kek(&key(7)).expect("derived"), &wrapped, data).is_ok());
    assert!(
        unwrap(&two.kek(&key(7)).expect("derived"), &wrapped, data).is_err(),
        "another salt"
    );
    assert!(
        unwrap(&one.kek(&key(8)).expect("derived"), &wrapped, data).is_err(),
        "another key"
    );
}

#[test]
fn a_customer_key_never_prints() {
    assert_eq!(format!("{:?}", key(7)), "CustomerKey(..)");
}
