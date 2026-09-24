//! Host normalization, the in-memory host map and its entries (host-routing; change
//! foundation D15).

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use axum::http::{HeaderMap, header};

use crate::config::BaseUrl;

/// A project's id, the only thing a project router is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectId(pub i64);

/// What a host maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEntry {
    Main,
    Project(ProjectId),
}

/// Normalizes a host name the way `Host` is: one trailing `:` plus digits removed,
/// ASCII letters lowercased, nothing else.
pub fn normalize(host: &str) -> String {
    let without_port = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    without_port.to_ascii_lowercase()
}

/// The normalized request host from exactly one non-empty `Host` header; `None` for a
/// missing, empty, repeated or non-visible-ASCII one.
pub fn request_host(headers: &HeaderMap) -> Option<String> {
    let mut values = headers.get_all(header::HOST).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let text = value.to_str().ok()?;
    if text.is_empty() {
        return None;
    }
    Some(normalize(text))
}

/// Normalized host → entry. Only the main host in this change; `projects` adds its
/// hosts.
#[derive(Debug, Clone)]
pub struct HostMap {
    entries: HashMap<String, HostEntry>,
}

impl HostMap {
    pub fn new(base_url: &BaseUrl) -> HostMap {
        let mut entries = HashMap::new();
        entries.insert(base_url.host().as_str().to_owned(), HostEntry::Main);
        HostMap { entries }
    }

    /// Adds a project host; the name must already be a validated DNS name.
    pub fn with_project(mut self, host: &str, project: ProjectId) -> HostMap {
        self.entries
            .insert(host.to_owned(), HostEntry::Project(project));
        self
    }

    pub fn get(&self, normalized_host: &str) -> Option<HostEntry> {
        self.entries.get(normalized_host).copied()
    }

    /// The configured project host names of `project`.
    pub fn project_host(&self, project: ProjectId) -> Option<&str> {
        self.entries
            .iter()
            .find(|(_, entry)| **entry == HostEntry::Project(project))
            .map(|(host, _)| host.as_str())
    }
}

/// The host map shared by every request; a rebuild swaps the whole map.
#[derive(Debug)]
pub struct SharedHostMap {
    map: RwLock<Arc<HostMap>>,
}

impl SharedHostMap {
    pub fn new(map: HostMap) -> SharedHostMap {
        SharedHostMap {
            map: RwLock::new(Arc::new(map)),
        }
    }

    pub fn current(&self) -> Arc<HostMap> {
        Arc::clone(&self.map.read().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn replace(&self, map: HostMap) {
        *self.map.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(map);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn base(url: &str) -> BaseUrl {
        crate::config::parse_base_url_for_tests(url)
    }

    fn headers(hosts: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for host in hosts {
            map.append(header::HOST, HeaderValue::from_str(host).unwrap());
        }
        map
    }

    #[test]
    fn normalization() {
        let map = HostMap::new(&base("https://kohaku.example.org:8443"));
        let lookup = |hosts: &[&str]| request_host(&headers(hosts)).and_then(|h| map.get(&h));
        assert_eq!(lookup(&["Kohaku.Example.ORG:8080"]), Some(HostEntry::Main));
        assert_eq!(lookup(&["kohaku.example.org"]), Some(HostEntry::Main));
        assert_eq!(lookup(&["kohaku.example.org:8443"]), Some(HostEntry::Main));
        for bad in [
            &["kohaku.example.org."][..],
            &["kohaku.example.org:x"],
            &["evil.example@kohaku.example.org"],
            &["kohaku.example.org", "evil.example"],
            &[],
            &[""],
            &["kohaku.example.org:"],
            &["kohaku:8080"],
            &["203.0.113.7"],
        ] {
            assert_eq!(lookup(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn lookalike_hosts_are_unknown() {
        let map = HostMap::new(&base("https://kohaku.example.org"));
        for host in [
            "evil.kohaku.example.org",
            "kohaku.example.org.evil.example",
            "bugs.example.net",
        ] {
            assert_eq!(map.get(&normalize(host)), None, "{host}");
        }
        let with_project = map.with_project("bugs.example.net", ProjectId(3));
        assert_eq!(
            with_project.get(&normalize("Bugs.Example.NET:443")),
            Some(HostEntry::Project(ProjectId(3)))
        );
    }
}
