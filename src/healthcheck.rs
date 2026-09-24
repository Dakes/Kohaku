//! `kohaku healthcheck`: one `GET /healthz` to the local server (operations:
//! Healthcheck command). Plain `std::net`: no proxy, no configuration, no database.

use std::fmt;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// The whole probe, connection included, must finish within this.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Why the server is unhealthy, in one line.
#[derive(Debug, PartialEq, Eq)]
pub enum Unhealthy {
    Refused,
    TimedOut,
    Malformed,
    Status(u16),
    Io(std::io::ErrorKind),
}

impl fmt::Display for Unhealthy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unhealthy::Refused => {
                f.write_str("unhealthy: connection refused, nothing listens on 127.0.0.1:8080")
            }
            Unhealthy::TimedOut => f.write_str("unhealthy: no answer within 5 s"),
            Unhealthy::Malformed => f.write_str("unhealthy: malformed response"),
            Unhealthy::Status(status) => write!(f, "unhealthy: status {status}"),
            Unhealthy::Io(kind) => write!(f, "unhealthy: {kind}"),
        }
    }
}

impl std::error::Error for Unhealthy {}

fn io_error(error: std::io::Error) -> Unhealthy {
    match error.kind() {
        std::io::ErrorKind::ConnectionRefused => Unhealthy::Refused,
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Unhealthy::TimedOut,
        kind => Unhealthy::Io(kind),
    }
}

/// Healthy if and only if `address` answers `GET /healthz` with 200 within
/// [`TIMEOUT`]; a redirect is never followed.
pub fn probe(address: SocketAddr) -> Result<(), Unhealthy> {
    let deadline = Instant::now() + TIMEOUT;
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(Unhealthy::TimedOut)
    };
    let mut stream = TcpStream::connect_timeout(&address, remaining()?).map_err(io_error)?;
    stream
        .set_write_timeout(Some(remaining()?))
        .map_err(io_error)?;
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nConnection: close\r\n\r\n")
        .map_err(io_error)?;
    let mut head = Vec::new();
    let mut buf = [0u8; 256];
    while !head.windows(2).any(|w| w == b"\r\n") {
        if head.len() > 1024 {
            return Err(Unhealthy::Malformed);
        }
        stream
            .set_read_timeout(Some(remaining()?))
            .map_err(io_error)?;
        match stream.read(&mut buf).map_err(io_error)? {
            0 => return Err(Unhealthy::Malformed),
            n => head.extend_from_slice(&buf[..n]),
        }
    }
    let status = head
        .strip_prefix(b"HTTP/1.1 ")
        .and_then(|rest| rest.get(..3))
        .and_then(|code| std::str::from_utf8(code).ok())
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(Unhealthy::Malformed)?;
    match status {
        200 => Ok(()),
        other => Err(Unhealthy::Status(other)),
    }
}
