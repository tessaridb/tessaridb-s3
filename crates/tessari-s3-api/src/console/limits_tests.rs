use tessari_s3_infrastructure::Live;

use super::Window;

#[test]
fn a_limit_changed_while_running_applies_to_the_next_admission() {
    let limit = Live::new(3);
    let window = Window::new(limit.clone());
    for second in 0..2 {
        assert_eq!(window.admit("ann", second), Ok(true));
    }
    // Lowered to what was already admitted: the next request in the minute is refused.
    assert!(limit.set(2));
    assert_eq!(window.admit("ann", 2), Ok(false));
    // Another key has its own window under the same limit.
    assert_eq!(window.admit("bob", 2), Ok(true));
}
