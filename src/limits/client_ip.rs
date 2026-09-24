//! The client address from the TCP peer and `X-Forwarded-For` (design §3 Client IP;
//! request-limits: Trusted proxies and the forwarded-for header).

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::http::HeaderValue;

use crate::config::TrustedProxies;

/// Something about the request worth one warning per server start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    XffFromUntrustedPeer,
    TrustedPeerSentNoXff,
    PrivateClient,
}

impl Warning {
    pub fn message(self) -> &'static str {
        match self {
            Warning::XffFromUntrustedPeer => "XFF from untrusted peer",
            Warning::TrustedPeerSentNoXff => "trusted peer sent no XFF",
            Warning::PrivateClient => "client address is loopback/RFC 1918/ULA/link-local",
        }
    }
}

/// The resolved address and what to warn about; the warnings never change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub client: IpAddr,
    pub warnings: Vec<Warning>,
}

/// `::ffff:a.b.c.d` as IPv4 `a.b.c.d`; every other address unchanged.
pub fn canonical(address: IpAddr) -> IpAddr {
    address.to_canonical()
}

/// Resolves the client address from the peer and every `X-Forwarded-For` line, in
/// received order.
pub fn resolve<'a>(
    peer: IpAddr,
    forwarded_for: impl IntoIterator<Item = &'a HeaderValue>,
    trusted: &TrustedProxies,
) -> Resolution {
    let peer = canonical(peer);
    let lines: Vec<&HeaderValue> = forwarded_for.into_iter().collect();
    let mut warnings = Vec::new();
    let client = if !trusted.contains(peer) {
        if !lines.is_empty() {
            warnings.push(Warning::XffFromUntrustedPeer);
        }
        peer
    } else if lines.is_empty() {
        warnings.push(Warning::TrustedPeerSentNoXff);
        peer
    } else {
        match parse_entries(&lines) {
            Some(entries) => entries
                .iter()
                .rev()
                .copied()
                .find(|&entry| !trusted.contains(entry))
                .unwrap_or(entries[0]),
            None => peer,
        }
    };
    if is_private(client) {
        warnings.push(Warning::PrivateClient);
    }
    Resolution { client, warnings }
}

/// Every entry of every line, canonicalized; `None` when any entry is malformed.
fn parse_entries(lines: &[&HeaderValue]) -> Option<Vec<IpAddr>> {
    let mut entries = Vec::new();
    for line in lines {
        let text = std::str::from_utf8(line.as_bytes()).ok()?;
        for entry in text.split(',') {
            let entry = entry.trim_matches([' ', '\t']);
            // std's parser refuses ports, brackets, zone indices, names and leading zeros.
            if entry.is_empty() || !entry.bytes().all(|b| b.is_ascii_graphic()) {
                return None;
            }
            entries.push(canonical(entry.parse().ok()?));
        }
    }
    Some(entries)
}

/// Loopback, RFC 1918, ULA or link-local.
pub fn is_private(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// One flag per warning: each is logged at most once per server start, without any
/// address.
#[derive(Debug, Default)]
pub struct ClientWarnings {
    logged: [AtomicBool; 3],
}

impl ClientWarnings {
    pub fn note(&self, warning: Warning) {
        let index = match warning {
            Warning::XffFromUntrustedPeer => 0,
            Warning::TrustedPeerSentNoXff => 1,
            Warning::PrivateClient => 2,
        };
        if !self.logged[index].swap(true, Ordering::Relaxed) {
            tracing::warn!("{}", warning.message());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted(list: &str) -> TrustedProxies {
        crate::config::parse_trusted_proxies_for_tests(list)
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn client(peer: &str, lines: &[&str], list: &str) -> IpAddr {
        let values: Vec<HeaderValue> = lines
            .iter()
            .map(|l| HeaderValue::from_bytes(l.as_bytes()).unwrap())
            .collect();
        resolve(ip(peer), &values, &trusted(list)).client
    }

    #[test]
    fn forged_header_without_a_trusted_peer() {
        assert_eq!(
            client("203.0.113.5", &["198.51.100.7"], "10.231.7.2"),
            ip("203.0.113.5")
        );
        assert_eq!(
            client("10.231.7.2", &["198.51.100.7"], "none"),
            ip("10.231.7.2")
        );
        assert_eq!(client("10.231.7.2", &[], "10.231.7.2"), ip("10.231.7.2"));
    }

    #[test]
    fn forged_entries_in_front_of_the_proxys() {
        let list = "10.231.7.2";
        let expected = ip("198.51.100.7");
        assert_eq!(
            client("10.231.7.2", &["192.0.2.1, 198.51.100.7"], list),
            expected
        );
        assert_eq!(
            client("10.231.7.2", &["10.231.7.2, 198.51.100.7"], list),
            expected
        );
        assert_eq!(
            client("10.231.7.2", &["192.0.2.1", "198.51.100.7"], list),
            expected
        );
    }

    #[test]
    fn chain_of_trusted_proxies() {
        let list = "10.231.7.0/29";
        assert_eq!(
            client("::ffff:10.231.7.2", &["198.51.100.7, 10.231.7.3"], list),
            ip("198.51.100.7")
        );
        assert_eq!(
            client("::ffff:10.231.7.2", &["10.231.7.3"], list),
            ip("10.231.7.3")
        );
    }

    #[test]
    fn malformed_entries() {
        let list = "10.231.7.2";
        let peer = ip("10.231.7.2");
        for line in [
            "unknown, 198.51.100.7",
            "192.0.2.1,,198.51.100.7",
            "198.51.100.7:4711",
            "[2001:db8::1]",
            "fe80::1%eth0",
            "example.org",
            "198.51.100.7,",
            "",
            "198.51.100.007",
        ] {
            assert_eq!(client("10.231.7.2", &[line], list), peer, "{line:?}");
        }
        let non_ascii = HeaderValue::from_bytes(b"198.51.100.7\xa0").unwrap();
        assert_eq!(resolve(peer, [&non_ascii], &trusted(list)).client, peer);
        assert_eq!(
            client("10.231.7.2", &["192.0.2.1 ,\t198.51.100.7 "], list),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn warnings() {
        let values = |s: &str| vec![HeaderValue::from_str(s).unwrap()];
        let list = trusted("10.231.7.2");
        let r = resolve(ip("203.0.113.5"), &values("192.0.2.1"), &list);
        assert_eq!(r.warnings, [Warning::XffFromUntrustedPeer]);
        let r = resolve(ip("10.231.7.2"), &values("198.51.100.7"), &list);
        assert!(r.warnings.is_empty());
        let r = resolve(ip("10.231.7.2"), &values("172.18.0.1"), &list);
        assert_eq!(r.warnings, [Warning::PrivateClient]);
        let r = resolve(ip("10.231.7.2"), &[], &list);
        assert_eq!(
            r.warnings,
            [Warning::TrustedPeerSentNoXff, Warning::PrivateClient]
        );
        for private in [
            "127.0.0.1",
            "::1",
            "10.1.2.3",
            "172.31.0.1",
            "192.168.1.1",
            "fd00::1",
            "169.254.1.1",
            "fe80::1",
        ] {
            assert!(is_private(ip(private)), "{private}");
        }
        for public in ["172.32.0.1", "198.51.100.7", "2001:db8::1", "fec0::1"] {
            assert!(!is_private(ip(public)), "{public}");
        }
    }
}
