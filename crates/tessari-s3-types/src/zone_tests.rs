use super::Zone;
use crate::Error;

#[test]
fn a_zone_is_a_short_lowercase_name() {
    for zone in ["eu-1a", "rack-07", "z", &"a".repeat(32)] {
        assert_eq!(
            Zone::new(zone).map(|named| named.as_str().to_owned()),
            Ok(zone.to_owned())
        );
    }
}

#[test]
fn anything_else_is_refused() {
    for zone in ["", "EU", "-a", "a-", "a_b", "a b", "a/b", &"a".repeat(33)] {
        assert_eq!(Zone::new(zone), Err(Error::InvalidZone), "{zone:?}");
    }
}
