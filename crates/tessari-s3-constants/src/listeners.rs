//! What every listener allows a client: how slowly it may send, and how many connections it may hold.

/// How long a connection to any listener may take to send a request's headers before it is closed, so a client that
/// trickles its headers cannot hold a connection open.
pub const SERVE_HEADER_READ_SECS: u64 = 30;

/// How long a handler waits for the next piece of a request body before the body fails, so a client that stops sending
/// mid-upload releases its request; time the handler spends elsewhere is not counted.
pub const SERVE_BODY_IDLE_SECS: u64 = 60;

/// Connections one listener holds open at once; the next is accepted when one closes, so a flood of idle connections
/// cannot exhaust the process's file descriptors.
pub const SERVE_MAX_CONNECTIONS: usize = 4096;
