//! Host normalization, the in-memory host map, its entries and its rebuilds
//! (host-routing; change foundation D15, change projects D4).

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, header};
use rusqlite::Connection;
use tokio::sync::watch;

use super::AppState;
use crate::config::BaseUrl;
use crate::db::migrate::DbFailure;

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

/// Normalized host → entry, and slug → custom domain for the canonical redirect;
/// built from `projects` (D4), so routing never queries the database.
#[derive(Debug, Clone)]
pub struct HostMap {
    entries: HashMap<String, HostEntry>,
    domains: HashMap<String, String>,
}

impl HostMap {
    /// The main host alone.
    pub fn new(base_url: &BaseUrl) -> HostMap {
        let mut entries = HashMap::new();
        entries.insert(base_url.host().as_str().to_owned(), HostEntry::Main);
        HostMap {
            entries,
            domains: HashMap::new(),
        }
    }

    /// Adds a project host; the name must already be a validated DNS name.
    pub fn with_project(mut self, host: &str, project: ProjectId) -> HostMap {
        self.entries
            .insert(host.to_owned(), HostEntry::Project(project));
        self
    }

    /// Adds project `slug`'s custom domain for the canonical redirect.
    pub fn with_domain(mut self, slug: &str, host: &str) -> HostMap {
        self.domains.insert(slug.to_owned(), host.to_owned());
        self
    }

    pub fn get(&self, normalized_host: &str) -> Option<HostEntry> {
        self.entries.get(normalized_host).copied()
    }

    /// The custom domain of the project with `slug`, if it has one.
    pub fn domain(&self, slug: &str) -> Option<&str> {
        self.domains.get(slug).map(String::as_str)
    }
}

/// The map of the main host and every project with a custom domain in `conn`.
pub fn load(conn: &Connection, base_url: &BaseUrl) -> rusqlite::Result<HostMap> {
    let mut statement =
        conn.prepare("SELECT id, slug, public_host FROM projects WHERE public_host IS NOT NULL")?;
    let rows = statement.query_map([], |row| {
        Ok((
            ProjectId(row.get(0)?),
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut map = HostMap::new(base_url);
    for row in rows {
        let (id, slug, host) = row?;
        map = map.with_project(&host, id).with_domain(&slug, &host);
    }
    Ok(map)
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

/// How often the watcher looks for changes made by another connection.
pub const WATCH_EVERY: Duration = Duration::from_secs(2);

/// A failed rebuild is logged at most this often.
const FAILURE_LOG_EVERY: Duration = Duration::from_secs(60);

/// Rebuilds the host map from the database on the watcher connection. Called after
/// every committed project change; a failure keeps the old map.
pub async fn refresh(app: &AppState) -> Result<(), DbFailure> {
    let (shared, base_url) = (Arc::clone(&app.host_map), app.base_url.clone());
    app.db
        .with_watcher(move |conn| {
            shared.replace(load(conn, &base_url)?);
            Ok(())
        })
        .await
}

/// Rebuilds the map whenever `PRAGMA data_version` shows a commit by another
/// connection, checking every [`WATCH_EVERY`] until `stop` changes.
pub async fn watch(app: AppState, mut stop: watch::Receiver<()>) {
    let mut seen: Option<i64> = None;
    let mut last_failure_log: Option<Instant> = None;
    let mut ticks = tokio::time::interval(WATCH_EVERY);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = stop.changed() => return,
            _ = ticks.tick() => {}
        }
        let (shared, base_url) = (Arc::clone(&app.host_map), app.base_url.clone());
        let result = app
            .db
            .with_watcher(move |conn| -> rusqlite::Result<i64> {
                let version: i64 = conn.query_row("PRAGMA data_version", [], |row| row.get(0))?;
                if seen != Some(version) {
                    shared.replace(load(conn, &base_url)?);
                }
                Ok(version)
            })
            .await;
        match result {
            // Only a successful rebuild moves on; a failed one is retried next time.
            Ok(version) => seen = Some(version),
            Err(error) => {
                if last_failure_log.is_none_or(|at| at.elapsed() >= FAILURE_LOG_EVERY) {
                    tracing::error!(
                        "cannot rebuild the host map, keeping the old one: {}",
                        DbFailure::from(error)
                    );
                    last_failure_log = Some(Instant::now());
                }
            }
        }
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
