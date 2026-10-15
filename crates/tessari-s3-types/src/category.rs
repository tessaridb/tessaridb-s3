//! The error taxonomy every crate of the workspace maps its failures into.

/// The category of a failure, shared by every crate so a boundary can route on it without reading messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    /// Input failed a schema, range or format check.
    Validation,
    /// The target does not exist.
    NotFound,
    /// A state precondition was violated.
    Conflict,
    /// The caller could not be authenticated.
    Unauthorized,
    /// The caller is authenticated and lacks permission.
    Forbidden,
    /// The operation exceeded its budget.
    Timeout,
    /// A dependency is not reachable.
    Unavailable,
    /// A defect or a broken invariant.
    Internal,
}
