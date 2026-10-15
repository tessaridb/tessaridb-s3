//! `tessaridb-s3 health`: is this node's S3 surface answering? It sends the one request any S3 server answers — an
//! unsigned `GET /` — to the local listener and passes on any HTTP status line, because a status line comes from the
//! server after its request pipeline ran rather than from the kernel's accept queue: a server that is listening but
//! wedged sends nothing and fails. The container's health check runs it, so the image needs no shell.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// How long the probe waits to connect, and then for the answer.
const PATIENCE: Duration = Duration::from_secs(5);

/// The local address to probe for a listener configured as `listen`: its port on loopback, since `0.0.0.0` is not an
/// address to connect to.
pub(crate) fn target(listen: &str) -> Result<SocketAddr, String> {
    let port = listen
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .ok_or_else(|| format!("the listen address {listen} names no port"))?;
    Ok(SocketAddr::from(([127, 0, 0, 1], port)))
}

/// `Ok` when the server at `address` answers `GET /` with an HTTP status line.
///
/// # Errors
/// What went wrong: no connection, nothing answered in time, or an answer that is not HTTP.
pub(crate) fn probe(address: SocketAddr) -> Result<(), String> {
    let mut stream = TcpStream::connect_timeout(&address, PATIENCE)
        .map_err(|error| format!("could not connect to {address}: {error}"))?;
    stream
        .set_read_timeout(Some(PATIENCE))
        .and_then(|()| stream.set_write_timeout(Some(PATIENCE)))
        .map_err(|error| format!("could not set a timeout: {error}"))?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|error| format!("could not send to {address}: {error}"))?;
    // A status line fits in the first 64 bytes; nothing past it is read.
    let mut answer = [0_u8; 64];
    let mut filled = 0;
    while filled < answer.len() {
        match stream.read(&mut answer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled = filled.saturating_add(read),
            Err(error) => return Err(format!("{address} answered nothing: {error}")),
        }
        if answer[..filled].contains(&b'\n') {
            break;
        }
    }
    let line = String::from_utf8_lossy(&answer[..filled]);
    let line = line.lines().next().unwrap_or_default();
    let status = line
        .strip_prefix("HTTP/1.1 ")
        .and_then(|rest| rest.get(..3))
        .filter(|code| {
            code.bytes().all(|b| b.is_ascii_digit()) && (b'1'..=b'5').contains(&code.as_bytes()[0])
        });
    match (status, line.is_empty()) {
        (Some(_), _) => Ok(()),
        (None, true) => Err(format!("{address} answered nothing")),
        (None, false) => Err(format!("{address} answered {line:?}, not HTTP")),
    }
}

/// Runs the probe against the listener `TESSARIDB_S3_LISTEN` names (or the default one).
///
/// # Errors
/// Why the server is not healthy.
pub(crate) async fn run() -> anyhow::Result<()> {
    let listen = std::env::var("TESSARIDB_S3_LISTEN")
        .unwrap_or_else(|_| tessari_s3_constants::DEFAULT_LISTEN.to_owned());
    let address = target(&listen).map_err(anyhow::Error::msg)?;
    tokio::task::spawn_blocking(move || probe(address))
        .await?
        .map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::{probe, target};

    /// A listener that answers its first connection with `answer` after reading the request.
    fn answering(answer: &'static [u8]) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = listener.local_addr().expect("an address");
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0_u8; 256];
                let _read = stream.read(&mut request);
                let _sent = stream.write_all(answer);
            }
        });
        address
    }

    #[test]
    fn a_server_answering_any_status_is_healthy() {
        let address = answering(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n");
        assert_eq!(probe(address), Ok(()));
    }

    #[test]
    fn a_listener_that_answers_nothing_is_not() {
        let address = answering(b"");
        assert!(probe(address).is_err_and(|why| why.contains("nothing")));
    }

    #[test]
    fn an_answer_that_is_not_http_is_not() {
        let address = answering(b"SSH-2.0-OpenSSH\r\n");
        assert!(probe(address).is_err_and(|why| why.contains("SSH-2.0")));
    }

    #[test]
    fn no_listener_is_not() {
        let address = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
            listener.local_addr().expect("an address")
        };
        assert!(probe(address).is_err_and(|why| why.contains("connect")));
    }

    #[test]
    fn the_probe_dials_the_configured_port_on_loopback() {
        assert_eq!(
            target("0.0.0.0:9100").map(|a| a.to_string()),
            Ok("127.0.0.1:9100".to_owned())
        );
        assert!(target("0.0.0.0").is_err());
    }
}
