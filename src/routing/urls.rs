//! Absolute URLs, built from configuration only (host-routing: Absolute URLs come only
//! from configured hosts). No function here takes a request.

use crate::config::BaseUrl;

#[derive(Debug, Clone)]
pub struct Urls {
    main_origin: String,
}

impl Urls {
    pub fn new(base_url: &BaseUrl) -> Urls {
        Urls {
            main_origin: base_url.origin(),
        }
    }

    /// The main-host origin: exactly `KOHAKU_BASE_URL`'s scheme, host and port.
    pub fn main_origin(&self) -> &str {
        &self.main_origin
    }

    /// A main-host URL; `path` starts with `/`.
    pub fn main(&self, path: &str) -> String {
        debug_assert!(path.starts_with('/'));
        format!("{}{path}", self.main_origin)
    }

    /// A token-bearing or unsubscribe link: always on the main host, whatever host the
    /// request that caused it used.
    pub fn token_link(&self, path: &str) -> String {
        self.main(path)
    }

    /// A project-host origin: `https://` plus the configured host.
    pub fn project_origin(host: &str) -> String {
        format!("https://{host}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_come_from_configuration() {
        let urls = Urls::new(&crate::config::parse_base_url_for_tests(
            "https://kohaku.example.org:8443",
        ));
        assert_eq!(urls.main("/x"), "https://kohaku.example.org:8443/x");
        assert_eq!(
            urls.token_link("/setup/t"),
            "https://kohaku.example.org:8443/setup/t"
        );
        assert_eq!(
            Urls::project_origin("bugs.example.net"),
            "https://bugs.example.net"
        );
    }
}
