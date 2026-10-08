//! The `console` domain: the signed session an operator holds after signing in to the console.

mod session;

pub use session::{Session, SessionError, issue, verify};
