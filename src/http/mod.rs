//! The HTTP pipeline around the routers (http-security; change foundation D10, D11,
//! D13, D14).

pub mod body;
pub mod errors;
pub mod fetch;
pub mod headers;
pub mod server;

use std::net::{IpAddr, SocketAddr};

use crate::routing::hosts::HostEntry;

/// The TCP peer, set by the server on every request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerAddr(pub SocketAddr);

/// The resolved client address (request-limits), set on host-routed requests only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAddr(pub IpAddr);

/// The host-map entry a request was routed by, with the origin a state-changing
/// request must come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedHost {
    pub entry: HostEntry,
    pub origin: String,
}
