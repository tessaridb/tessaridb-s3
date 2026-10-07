//! Which operations this server implements. Every other catalog operation is answered `NotImplemented` (501).

use super::catalog::Operation;

/// Operations with a handler. Empty until the object core lands: every request is refused with 501.
const IMPLEMENTED: &[Operation] = &[];

/// Whether `operation` has a handler; `false` means the request is answered `NotImplemented`.
#[must_use]
pub fn is_implemented(operation: Operation) -> bool {
    IMPLEMENTED.contains(&operation)
}
