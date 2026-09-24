//! Configuration from environment variables only, secrets included, read once when a
//! command starts (configuration spec; change foundation D5).
//!
//! Values are taken exactly as given: never trimmed, case-folded or repaired. Every
//! problem of one run is reported together, each naming the variable and the broken
//! rule; no message ever contains a value, so a secret cannot leak through one.

use std::ffi::OsString;
use std::fmt;
use std::net::IpAddr;

use crate::cli::Command;
use crate::keys::{INSTANCE_SECRET_MIN_BYTES, InstanceSecret};

pub const BASE_URL: &str = "KOHAKU_BASE_URL";
pub const TRUSTED_PROXIES: &str = "KOHAKU_TRUSTED_PROXIES";
pub const SECRET: &str = "KOHAKU_SECRET";
pub const SMTP_HOST: &str = "KOHAKU_SMTP_HOST";
pub const SMTP_PORT: &str = "KOHAKU_SMTP_PORT";
pub const SMTP_TLS: &str = "KOHAKU_SMTP_TLS";
pub const SMTP_USERNAME: &str = "KOHAKU_SMTP_USERNAME";
pub const SMTP_PASSWORD: &str = "KOHAKU_SMTP_PASSWORD";
pub const SMTP_FROM: &str = "KOHAKU_SMTP_FROM";
pub const PUBLIC_MAIL_PER_HOUR: &str = "KOHAKU_PUBLIC_MAIL_PER_HOUR";

/// The SMTP connection settings; a `dev` build prints mail instead and refuses them.
pub const SMTP_CONNECTION_SETTINGS: [&str; 5] =
    [SMTP_HOST, SMTP_PORT, SMTP_TLS, SMTP_USERNAME, SMTP_PASSWORD];

/// `KOHAKU_PUBLIC_MAIL_PER_HOUR` when unset.
pub const DEFAULT_PUBLIC_MAIL_PER_HOUR: u32 = 60;

/// The placeholders `.env.example` ships; `serve` refuses each unchanged.
pub mod placeholders {
    /// `KOHAKU_DOMAIN`, from which `docker-compose.yml` builds `KOHAKU_BASE_URL`.
    pub const DOMAIN: &str = "bugs.example.com";
    pub const BASE_URL: &str = "https://bugs.example.com";
    pub const SMTP_HOST: &str = "smtp.example.com";
    pub const SMTP_USERNAME: &str = "your-smtp-username";
    pub const SMTP_FROM: &str = "bugs@example.com";
}

/// Reads one variable of the process environment; the only place Kohaku does.
#[expect(
    clippy::disallowed_methods,
    reason = "config.rs is the one module that reads the environment"
)]
pub fn process_environment(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The settings a command needs; `restore`, `restore --list` and `healthcheck` need none.
pub enum Config {
    Serve(ServeConfig),
    /// `backup` and `admin unlock`.
    Secret(SecretConfig),
    /// `admin reset-password`, which prints a link.
    ResetLink(ResetLinkConfig),
    None,
}

pub struct ServeConfig {
    pub base_url: BaseUrl,
    pub trusted_proxies: TrustedProxies,
    pub secret: InstanceSecret,
    pub sender: SenderAddress,
    pub transport: MailTransport,
    pub public_mail_per_hour: u32,
}

pub struct SecretConfig {
    pub secret: InstanceSecret,
}

pub struct ResetLinkConfig {
    pub secret: InstanceSecret,
    pub base_url: BaseUrl,
}

/// How the outbox worker delivers mail in this build.
pub enum MailTransport {
    Smtp(SmtpConfig),
    /// A `dev` build prints every mail to the terminal instead of sending it.
    #[cfg(feature = "dev")]
    Print,
}

pub struct SmtpConfig {
    pub host: DnsName,
    pub port: u16,
    pub tls: SmtpTls,
    pub username: String,
    pub password: SmtpPassword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    /// TLS from the first byte.
    Implicit,
    /// STARTTLS required before authentication and any mail data.
    Starttls,
}

/// `KOHAKU_SMTP_PASSWORD`. Implements neither `Debug` nor `Display`.
pub struct SmtpPassword(String);

impl SmtpPassword {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// One broken rule, naming the variable; never holds the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub variable: &'static str,
    pub rule: &'static str,
}

/// Every problem found in one run.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub problems: Vec<Problem>,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, problem) in self.problems.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{}: {}", problem.variable, problem.rule)?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

const MISSING: &str = "is required and has no default; set it (see the README)";
const NOT_UTF8: &str = "is not valid UTF-8";
const UNCHANGED_EXAMPLE: &str = "still has the example value from .env.example; set your own";
#[cfg(feature = "dev")]
const DEV_BUILD_SENDS_NO_MAIL: &str =
    "must not be set in a dev build, which prints mail to the terminal instead of sending it";

impl Config {
    /// Loads exactly the settings `command` needs from `lookup` (the process
    /// environment in the binary, a map in tests).
    pub fn load<F>(command: &Command, lookup: F) -> Result<Config, ConfigError>
    where
        F: Fn(&str) -> Option<OsString>,
    {
        let mut reader = Reader {
            lookup,
            problems: Vec::new(),
        };
        let config = match command {
            Command::Serve => reader.serve().map(Config::Serve),
            Command::Backup(_) | Command::AdminUnlock(_) => {
                reader.secret_only().map(Config::Secret)
            }
            Command::AdminResetPassword(_) => reader.reset_link().map(Config::ResetLink),
            Command::Restore(_) | Command::RestoreList | Command::Healthcheck => Some(Config::None),
        };
        match config {
            Some(config) if reader.problems.is_empty() => Ok(config),
            _ => Err(ConfigError {
                problems: reader.problems,
            }),
        }
    }
}

/// Loads only the SMTP connection settings, by the `serve` rules. Tests of the SMTP
/// adapter use it in every build; `serve` loads them through [`Config::load`].
pub fn load_smtp<F>(lookup: F) -> Result<SmtpConfig, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let mut reader = Reader {
        lookup,
        problems: Vec::new(),
    };
    match reader.smtp() {
        Some(config) if reader.problems.is_empty() => Ok(config),
        _ => Err(ConfigError {
            problems: reader.problems,
        }),
    }
}

struct Reader<F> {
    lookup: F,
    problems: Vec<Problem>,
}

impl<F> Reader<F>
where
    F: Fn(&str) -> Option<OsString>,
{
    fn problem(&mut self, variable: &'static str, rule: &'static str) {
        self.problems.push(Problem { variable, rule });
    }

    /// `None` (unset) or the UTF-8 value, empty included.
    fn raw(&mut self, name: &'static str) -> Option<String> {
        let value = (self.lookup)(name)?;
        match value.into_string() {
            Ok(value) => Some(value),
            Err(_) => {
                self.problem(name, NOT_UTF8);
                None
            }
        }
    }

    /// A required setting: unset or empty counts as missing.
    fn required<T>(
        &mut self,
        name: &'static str,
        parse: fn(&str) -> Result<T, &'static str>,
    ) -> Option<T> {
        let is_set = (self.lookup)(name).is_some();
        match self.raw(name) {
            Some(value) if !value.is_empty() => self.parsed(name, &value, parse),
            Some(_) => {
                self.problem(name, MISSING);
                None
            }
            None if !is_set => {
                self.problem(name, MISSING);
                None
            }
            // Set but not UTF-8: already reported.
            None => None,
        }
    }

    fn parsed<T>(
        &mut self,
        name: &'static str,
        value: &str,
        parse: fn(&str) -> Result<T, &'static str>,
    ) -> Option<T> {
        match parse(value) {
            Ok(parsed) => Some(parsed),
            Err(rule) => {
                self.problem(name, rule);
                None
            }
        }
    }

    fn refuse_placeholder(&mut self, name: &'static str, placeholder: &str) -> bool {
        let unchanged = matches!((self.lookup)(name), Some(value) if value == placeholder);
        if unchanged {
            self.problem(name, UNCHANGED_EXAMPLE);
        }
        unchanged
    }

    /// A required setting that must not equal its `.env.example` placeholder.
    fn required_not_placeholder<T>(
        &mut self,
        name: &'static str,
        placeholder: &str,
        parse: fn(&str) -> Result<T, &'static str>,
    ) -> Option<T> {
        if self.refuse_placeholder(name, placeholder) {
            return None;
        }
        self.required(name, parse)
    }

    fn secret(&mut self) -> Option<InstanceSecret> {
        self.required(SECRET, parse_instance_secret)
    }

    fn secret_only(&mut self) -> Option<SecretConfig> {
        let secret = self.secret()?;
        Some(SecretConfig { secret })
    }

    fn reset_link(&mut self) -> Option<ResetLinkConfig> {
        let secret = self.secret();
        let base_url =
            self.required_not_placeholder(BASE_URL, placeholders::BASE_URL, parse_base_url);
        Some(ResetLinkConfig {
            secret: secret?,
            base_url: base_url?,
        })
    }

    fn serve(&mut self) -> Option<ServeConfig> {
        let base_url =
            self.required_not_placeholder(BASE_URL, placeholders::BASE_URL, parse_base_url);
        let trusted_proxies = self.required(TRUSTED_PROXIES, parse_trusted_proxies);
        let secret = self.secret();
        let sender =
            self.required_not_placeholder(SMTP_FROM, placeholders::SMTP_FROM, parse_sender);
        let transport = self.transport();
        let public_mail_per_hour = match self.raw(PUBLIC_MAIL_PER_HOUR) {
            None if self.problems_for(PUBLIC_MAIL_PER_HOUR) => None,
            None => Some(DEFAULT_PUBLIC_MAIL_PER_HOUR),
            Some(value) => self.parsed(PUBLIC_MAIL_PER_HOUR, &value, parse_public_mail_per_hour),
        };
        Some(ServeConfig {
            base_url: base_url?,
            trusted_proxies: trusted_proxies?,
            secret: secret?,
            sender: sender?,
            transport: transport?,
            public_mail_per_hour: public_mail_per_hour?,
        })
    }

    fn problems_for(&self, name: &str) -> bool {
        self.problems.iter().any(|problem| problem.variable == name)
    }

    #[cfg(not(feature = "dev"))]
    fn transport(&mut self) -> Option<MailTransport> {
        self.smtp().map(MailTransport::Smtp)
    }

    #[cfg(feature = "dev")]
    fn transport(&mut self) -> Option<MailTransport> {
        let mut refused = false;
        for name in SMTP_CONNECTION_SETTINGS {
            if (self.lookup)(name).is_some() {
                self.problem(name, DEV_BUILD_SENDS_NO_MAIL);
                refused = true;
            }
        }
        (!refused).then_some(MailTransport::Print)
    }

    fn smtp(&mut self) -> Option<SmtpConfig> {
        let host =
            self.required_not_placeholder(SMTP_HOST, placeholders::SMTP_HOST, parse_smtp_host);
        let port = self.required(SMTP_PORT, parse_smtp_port);
        let tls = self.required(SMTP_TLS, parse_smtp_tls);
        let username = self.required_not_placeholder(
            SMTP_USERNAME,
            placeholders::SMTP_USERNAME,
            parse_smtp_username,
        );
        let password = self.required(SMTP_PASSWORD, parse_smtp_password);
        Some(SmtpConfig {
            host: host?,
            port: port?,
            tls: tls?,
            username: username?,
            password: password?,
        })
    }
}

/// A lowercase ASCII DNS name (configuration: Base URL format): labels of 1–63
/// characters from `a`–`z`, `0`–`9` and `-`, none starting or ending with `-`, at most
/// 253 characters, no trailing dot, no IP literal. One validator for every host Kohaku
/// is configured with.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DnsName(String);

impl DnsName {
    pub const RULE: &'static str = "must be a lowercase ASCII DNS name (letters, digits, '-' and \
        dots; internationalized names in punycode), without trailing dot, port or scheme, \
        and not an IP address";

    pub fn parse(text: &str) -> Result<DnsName, &'static str> {
        if text.is_empty() || text.len() > 253 {
            return Err(Self::RULE);
        }
        let mut last = "";
        for label in text.split('.') {
            let allowed = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
            if label.is_empty()
                || label.len() > 63
                || !label.bytes().all(allowed)
                || label.starts_with('-')
                || label.ends_with('-')
            {
                return Err(Self::RULE);
            }
            last = label;
        }
        // No top-level domain is numeric, so this refuses IPv4 literals and lookalikes.
        if last.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Self::RULE);
        }
        Ok(DnsName(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DnsName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `KOHAKU_BASE_URL`: the main host's origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseUrl {
    https: bool,
    host: DnsName,
    port: Option<u16>,
}

impl BaseUrl {
    pub fn host(&self) -> &DnsName {
        &self.host
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Scheme, host and optional port, exactly as a browser sends it in `Origin`.
    pub fn origin(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        match self.port {
            Some(port) => format!("{scheme}://{}:{port}", self.host),
            None => format!("{scheme}://{}", self.host),
        }
    }
}

const BASE_URL_FORM: &str = "must be exactly https://host[:port]: no path (not even /), query, \
    fragment or user information";
const BASE_URL_SCHEME: &str = if cfg!(feature = "dev") {
    "must start with https:// (a dev build also accepts http://localhost[:port])"
} else {
    "must start with https://"
};
const BASE_URL_PORT: &str = "port must be 1-65535 without leading zeros, and not the scheme's default (443, or 80 for http)";
const BASE_URL_HTTP_HOST: &str = "http:// is accepted only for localhost, and only in a dev build";

fn parse_base_url(text: &str) -> Result<BaseUrl, &'static str> {
    let (https, rest) = if let Some(rest) = text.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = text.strip_prefix("http://")
        && cfg!(feature = "dev")
    {
        (false, rest)
    } else {
        return Err(BASE_URL_SCHEME);
    };
    if rest.contains(['/', '?', '#', '@']) {
        return Err(BASE_URL_FORM);
    }
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(parse_port(port).ok_or(BASE_URL_PORT)?)),
        None => (rest, None),
    };
    let default_port = if https { 443 } else { 80 };
    if port == Some(default_port) {
        return Err(BASE_URL_PORT);
    }
    let host = DnsName::parse(host)?;
    if !https && host.as_str() != "localhost" {
        return Err(BASE_URL_HTTP_HOST);
    }
    Ok(BaseUrl { https, host, port })
}

/// Parses a `KOHAKU_BASE_URL` value that tests know to be valid.
#[cfg(test)]
pub(crate) fn parse_base_url_for_tests(text: &str) -> BaseUrl {
    parse_base_url(text).expect("valid test base URL")
}

/// A decimal without sign, spaces or leading zeros.
fn parse_decimal(text: &str) -> Option<u32> {
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text.len() == 1 || !text.starts_with('0'));
    if canonical { text.parse().ok() } else { None }
}

fn parse_port(text: &str) -> Option<u16> {
    parse_decimal(text)
        .and_then(|n| u16::try_from(n).ok())
        .filter(|&n| n != 0)
}

/// `KOHAKU_TRUSTED_PROXIES`: the peers whose `X-Forwarded-For` Kohaku believes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedProxies {
    /// `none`: no peer is trusted.
    None,
    List(Vec<IpNet>),
}

/// An exact address (full-length prefix) or a CIDR block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    address: IpAddr,
    prefix: u8,
}

impl IpNet {
    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.address, address) {
            (IpAddr::V4(net), IpAddr::V4(addr)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(net) & mask == u32::from(addr) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(addr)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(net) & mask == u128::from(addr) & mask
            }
            _ => false,
        }
    }
}

impl TrustedProxies {
    /// Whether `peer`, already canonicalized (an IPv4-mapped IPv6 address as IPv4),
    /// is a trusted proxy.
    pub fn contains(&self, peer: IpAddr) -> bool {
        match self {
            TrustedProxies::None => false,
            TrustedProxies::List(nets) => nets.iter().any(|net| net.contains(peer)),
        }
    }
}

const TRUSTED_PROXIES_RULE: &str = "must be `none` or a comma-separated list, without spaces or \
    empty entries, of exact IP addresses (normally) or CIDR blocks such as 10.231.7.0/29 \
    (last resort); no IPv4-mapped IPv6, no zone index, no /0, no bits set past the prefix";

fn parse_trusted_proxies(text: &str) -> Result<TrustedProxies, &'static str> {
    if text == "none" {
        return Ok(TrustedProxies::None);
    }
    text.split(',')
        .map(parse_ip_net)
        .collect::<Option<Vec<_>>>()
        .map(TrustedProxies::List)
        .ok_or(TRUSTED_PROXIES_RULE)
}

/// Parses a `KOHAKU_TRUSTED_PROXIES` value that tests know to be valid.
#[cfg(test)]
pub(crate) fn parse_trusted_proxies_for_tests(text: &str) -> TrustedProxies {
    parse_trusted_proxies(text).expect("valid test list")
}

fn parse_ip_net(entry: &str) -> Option<IpNet> {
    let (address, prefix) = match entry.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (entry, None),
    };
    // std refuses leading zeros, zone indices and brackets.
    let address: IpAddr = address.parse().ok()?;
    let max = if address.is_ipv4() { 32 } else { 128 };
    let prefix = match prefix {
        Some(text) => u8::try_from(parse_decimal(text)?)
            .ok()
            .filter(|&p| (1..=max).contains(&p))?,
        None => max,
    };
    let net = IpNet { address, prefix };
    let host_bits_clear = match address {
        IpAddr::V4(v4) => {
            u32::from(v4) & !u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0) == 0
        }
        IpAddr::V6(v6) => {
            u128::from(v6) & !u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0) == 0
        }
    };
    let mapped = matches!(address, IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some() && prefix >= 96);
    (host_bits_clear && !mapped).then_some(net)
}

fn parse_instance_secret(text: &str) -> Result<InstanceSecret, &'static str> {
    const RULE: &str = "must be standard base64 with = padding (RFC 4648 §4) on one line, without \
        spaces, decoding to at least 32 random bytes; generate one with \
        `head -c 32 /dev/urandom | base64`";
    const _: () = assert!(INSTANCE_SECRET_MIN_BYTES == 32, "RULE names 32 bytes");
    InstanceSecret::from_base64(text).map_err(|_| RULE)
}

fn parse_smtp_host(text: &str) -> Result<DnsName, &'static str> {
    DnsName::parse(text)
}

fn parse_smtp_port(text: &str) -> Result<u16, &'static str> {
    parse_port(text).ok_or("must be a port number 1-65535 without sign, spaces or leading zeros")
}

fn parse_smtp_tls(text: &str) -> Result<SmtpTls, &'static str> {
    match text {
        "implicit" => Ok(SmtpTls::Implicit),
        "starttls" => Ok(SmtpTls::Starttls),
        _ => Err(
            "must be `implicit` (TLS from the first byte, usually port 465) or `starttls` \
            (usually port 587); mail is never sent without TLS",
        ),
    }
}

const NO_CONTROL_CHARACTERS: &str = "must not contain control characters";

fn parse_smtp_username(text: &str) -> Result<String, &'static str> {
    if text.chars().any(char::is_control) {
        return Err(NO_CONTROL_CHARACTERS);
    }
    Ok(text.to_owned())
}

fn parse_smtp_password(text: &str) -> Result<SmtpPassword, &'static str> {
    parse_smtp_username(text).map(SmtpPassword)
}

/// `KOHAKU_SMTP_FROM`: the sender of every mail, a bare `local@domain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderAddress(String);

impl SenderAddress {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const SENDER_RULE: &str = "must be one bare address local@domain of at most 254 characters: a \
    dot-atom local part (letters, digits and !#$%&'*+/=?^_`{|}~- separated by single dots), a \
    domain under the DNS name rules, no display name or angle brackets";

fn parse_sender(text: &str) -> Result<SenderAddress, &'static str> {
    if text.len() > 254 {
        return Err(SENDER_RULE);
    }
    let (local, domain) = text.split_once('@').ok_or(SENDER_RULE)?;
    let atext = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~-".contains(c);
    let dot_atom = !local.is_empty()
        && local
            .split('.')
            .all(|atom| !atom.is_empty() && atom.chars().all(atext));
    if !dot_atom || DnsName::parse(domain).is_err() {
        return Err(SENDER_RULE);
    }
    Ok(SenderAddress(text.to_owned()))
}

/// Parses a sender that tests know to be valid.
#[cfg(test)]
pub(crate) fn parse_sender_for_tests(text: &str) -> SenderAddress {
    parse_sender(text).expect("valid test sender")
}

fn parse_public_mail_per_hour(text: &str) -> Result<u32, &'static str> {
    parse_decimal(text)
        .filter(|&n| n >= 1)
        .ok_or("must be a whole number 1-4294967295 without sign, spaces or leading zeros (unset means 60)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{BackupTarget, RestoreSource};
    use crate::test_support::{TEST_SECRET, valid_environment};
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, OsString> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect()
    }

    fn load(command: &Command, vars: &HashMap<String, OsString>) -> Result<Config, ConfigError> {
        Config::load(command, |name| vars.get(name).cloned())
    }

    fn serve_problems(vars: &HashMap<String, OsString>) -> Vec<Problem> {
        match load(&Command::Serve, vars) {
            Ok(_) => Vec::new(),
            Err(error) => error.problems,
        }
    }

    fn variables(problems: &[Problem]) -> Vec<&'static str> {
        problems.iter().map(|p| p.variable).collect()
    }

    fn with(changes: &[(&str, Option<&str>)]) -> HashMap<String, OsString> {
        let mut vars = valid_environment();
        for (name, value) in changes {
            match value {
                Some(value) => vars.insert(name.to_string(), OsString::from(value)),
                None => vars.remove(*name),
            };
        }
        vars
    }

    #[test]
    fn base_url_values() {
        let accepted = ["https://bugs.kohaku.test", "https://bugs.kohaku.test:8443"];
        for value in accepted {
            assert!(parse_base_url(value).is_ok(), "{value}");
        }
        assert_eq!(
            parse_base_url("http://localhost:8080").is_ok(),
            cfg!(feature = "dev")
        );
        let rejected = [
            "https://bugs.kohaku.test/",
            "https://bugs.kohaku.test/kohaku",
            "https://bugs.kohaku.test?x=1",
            "https://bugs.kohaku.test#x",
            "https://user@bugs.kohaku.test",
            "HTTPS://bugs.kohaku.test",
            "https://Bugs.Kohaku.test",
            "https://bugs.kohaku.test.",
            "https://bugs.kohaku.test:443",
            "https://bugs.kohaku.test:0443",
            "https://bugs.kohaku.test:0",
            "https://bugs.kohaku.test:65536",
            "https://bugs.kohaku.test:",
            "https://192.0.2.10",
            "https://[2001:db8::1]",
            "http://bugs.kohaku.test",
            "http://127.0.0.1:8080",
            "http://localhost:80",
            "https://",
            "bugs.kohaku.test",
        ];
        for value in rejected {
            assert!(parse_base_url(value).is_err(), "{value}");
        }
        for value in [
            "https://bugs.kohaku.test/",
            "https://bugs.kohaku.test?x=1",
            "https://user@bugs.kohaku.test",
        ] {
            assert!(
                parse_base_url(value)
                    .unwrap_err()
                    .contains("https://host[:port]"),
                "{value}"
            );
        }
        assert_eq!(
            parse_base_url("https://bugs.kohaku.test:8443")
                .unwrap()
                .origin(),
            "https://bugs.kohaku.test:8443"
        );
        assert_eq!(
            parse_base_url("https://xn--bcher-kva.example")
                .unwrap()
                .origin(),
            "https://xn--bcher-kva.example"
        );
    }

    #[test]
    fn dns_names() {
        for name in [
            "localhost",
            "a.b",
            "xn--bcher-kva.example",
            "a-b.c1",
            &format!("{}.test", "a".repeat(63)),
        ] {
            assert!(DnsName::parse(name).is_ok(), "{name}");
        }
        let long = format!("{}.test", vec!["a".repeat(63); 4].join("."));
        for name in [
            "",
            "a..b",
            ".a",
            "-a.test",
            "a-.test",
            "A.test",
            "a_b.test",
            "a.test.",
            "1.2.3.4",
            "a.123",
            &format!("{}.test", "a".repeat(64)),
            &long,
            "ä.test",
            "a b.test",
        ] {
            assert!(DnsName::parse(name).is_err(), "{name}");
        }
    }

    #[test]
    fn trusted_proxy_lists() {
        let accepted = [
            "10.231.7.2,fd4b:7a1c:2e90:1::2",
            "none",
            "10.231.7.0/29",
            "fd4b:7a1c:2e90:1::/64",
        ];
        for value in accepted {
            assert!(parse_trusted_proxies(value).is_ok(), "{value}");
        }
        let rejected = [
            "10.231.7.2, fd00::2",
            "10.231.7.2,,10.231.7.3",
            "010.231.7.2",
            "fe80::1%eth0",
            "::ffff:10.231.7.2",
            "::ffff:10.231.7.0/120",
            "caddy",
            "none,10.231.7.2",
            "NONE",
            "10.231.7.2/24",
            "0.0.0.0/0",
            "::/0",
            "10.231.7.0/029",
            "10.231.7.0/33",
            "fd00::/129",
            "",
            ",",
            "10.231.7.2,",
        ];
        for value in rejected {
            assert!(parse_trusted_proxies(value).is_err(), "{value}");
        }
    }

    #[test]
    fn trusted_set_matches_exact_addresses_and_blocks() {
        let exact = parse_trusted_proxies("10.231.7.2,fd4b:7a1c:2e90:1::2").unwrap();
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(exact.contains(ip("10.231.7.2")));
        assert!(exact.contains(ip("fd4b:7a1c:2e90:1::2")));
        assert!(!exact.contains(ip("10.231.7.3")));
        assert!(!exact.contains(ip("fd4b:7a1c:2e90:1::3")));
        let block = parse_trusted_proxies("10.231.7.0/29,fd4b:7a1c:2e90:1::/64").unwrap();
        assert!(block.contains(ip("10.231.7.7")));
        assert!(!block.contains(ip("10.231.7.8")));
        assert!(block.contains(ip("fd4b:7a1c:2e90:1:ffff::1")));
        assert!(!block.contains(ip("fd4b:7a1c:2e90:2::1")));
        assert!(
            !parse_trusted_proxies("none")
                .unwrap()
                .contains(ip("10.231.7.2"))
        );
        assert!(
            !parse_trusted_proxies("10.0.0.0/8")
                .unwrap()
                .contains(ip("::ffff:10.0.0.1"))
        );
    }

    #[test]
    fn smtp_values() {
        for value in ["none", "off", "STARTTLS", ""] {
            assert!(parse_smtp_tls(value).is_err(), "{value}");
        }
        assert!(parse_smtp_tls("smtp").unwrap_err().contains("`implicit`"));
        assert!(parse_smtp_tls("smtp").unwrap_err().contains("`starttls`"));
        for value in ["0", "65536", "smtp", " 587", "0587", "+587"] {
            assert!(parse_smtp_port(value).is_err(), "{value}");
        }
        assert_eq!(parse_smtp_port("587"), Ok(587));
        for value in [
            "smtp://smtp.kohaku.test",
            "smtp.kohaku.test:587",
            "smtp.kohaku.test/x",
            "192.0.2.1",
        ] {
            assert!(parse_smtp_host(value).is_err(), "{value}");
        }
        for value in [
            "Kohaku <kohaku@bugs.kohaku.test>",
            "a@b@bugs.kohaku.test",
            "@bugs.kohaku.test",
            "kohaku@",
            "a b@x.test",
            "a..b@x.test",
            ".a@x.test",
            "a@X.test",
            "a@x.test\n",
            "<a@x.test>",
            &format!("{}@x.test", "a".repeat(250)),
        ] {
            assert!(parse_sender(value).is_err(), "{value}");
        }
        for value in [
            "kohaku@bugs.kohaku.test",
            "kohaku@localhost",
            "a.b+c@x.test",
        ] {
            assert!(parse_sender(value).is_ok(), "{value}");
        }
        assert!(parse_smtp_username("user\u{7}").is_err());
        assert!(parse_smtp_password("pass\nword").is_err());
        assert_eq!(
            parse_smtp_password("correct horse battery staple")
                .unwrap()
                .expose(),
            "correct horse battery staple"
        );
    }

    #[test]
    fn public_mail_budget_values() {
        assert_eq!(parse_public_mail_per_hour("120"), Ok(120));
        assert_eq!(parse_public_mail_per_hour("4294967295"), Ok(u32::MAX));
        for value in ["0", "-1", "060", "60/h", "4294967296", "", " 60"] {
            assert!(parse_public_mail_per_hour(value).is_err(), "{value}");
        }
        let Ok(Config::Serve(serve)) =
            load(&Command::Serve, &with(&[(PUBLIC_MAIL_PER_HOUR, None)]))
        else {
            panic!("valid environment must load");
        };
        assert_eq!(serve.public_mail_per_hour, 60);
        let Ok(Config::Serve(serve)) = load(
            &Command::Serve,
            &with(&[(PUBLIC_MAIL_PER_HOUR, Some("120"))]),
        ) else {
            panic!("valid environment must load");
        };
        assert_eq!(serve.public_mail_per_hour, 120);
        assert_eq!(
            variables(&serve_problems(&with(&[(PUBLIC_MAIL_PER_HOUR, Some(""))]))),
            [PUBLIC_MAIL_PER_HOUR]
        );
    }

    #[test]
    fn valid_environment_loads() {
        assert!(serve_problems(&valid_environment()).is_empty());
    }

    #[test]
    fn every_problem_is_reported_in_one_run() {
        let problems = serve_problems(&with(&[
            (TRUSTED_PROXIES, None),
            (SMTP_FROM, Some("")),
            (BASE_URL, Some("https://bugs.kohaku.test/")),
        ]));
        assert_eq!(variables(&problems), [BASE_URL, TRUSTED_PROXIES, SMTP_FROM]);
        assert_eq!(problems[1].rule, MISSING);
        assert_eq!(problems[2].rule, MISSING);
    }

    #[cfg(not(feature = "dev"))]
    #[test]
    fn every_problem_is_reported_in_one_run_smtp() {
        let problems = serve_problems(&with(&[
            (TRUSTED_PROXIES, None),
            (SMTP_FROM, Some("")),
            (SMTP_PORT, Some(" 587")),
        ]));
        assert_eq!(
            variables(&problems),
            [TRUSTED_PROXIES, SMTP_FROM, SMTP_PORT]
        );
    }

    #[cfg(not(feature = "dev"))]
    #[test]
    fn malformed_smtp_values_rejected() {
        let problems = serve_problems(&with(&[
            (SMTP_TLS, Some("STARTTLS")),
            (SMTP_PORT, Some("smtp")),
            (SMTP_HOST, Some("smtp.kohaku.test:587")),
            (SMTP_FROM, Some("Kohaku <kohaku@bugs.kohaku.test>")),
        ]));
        let mut names = variables(&problems);
        names.sort_unstable();
        assert_eq!(names, [SMTP_FROM, SMTP_HOST, SMTP_PORT, SMTP_TLS]);
    }

    #[cfg(not(feature = "dev"))]
    #[test]
    fn unreachable_smtp_values_are_accepted() {
        let vars = with(&[
            (SMTP_HOST, Some("smtp.kohaku.test")),
            (SMTP_PORT, Some("587")),
            (SMTP_TLS, Some("starttls")),
            (SMTP_USERNAME, Some("kohaku")),
            (SMTP_PASSWORD, Some("correct horse battery staple")),
            (SMTP_FROM, Some("kohaku@bugs.kohaku.test")),
        ]);
        let Ok(Config::Serve(serve)) = load(&Command::Serve, &vars) else {
            panic!("must load")
        };
        let MailTransport::Smtp(smtp) = serve.transport;
        assert_eq!(
            (smtp.host.as_str(), smtp.port, smtp.tls),
            ("smtp.kohaku.test", 587, SmtpTls::Starttls)
        );
        assert_eq!(smtp.password.expose(), "correct horse battery staple");
    }

    #[cfg(feature = "dev")]
    #[test]
    fn development_build_takes_no_smtp_server() {
        let mut vars = valid_environment();
        for name in SMTP_CONNECTION_SETTINGS {
            assert!(!vars.contains_key(name), "dev test environment sets {name}");
        }
        let Ok(Config::Serve(serve)) = load(&Command::Serve, &vars) else {
            panic!("must load")
        };
        assert!(matches!(serve.transport, MailTransport::Print));
        vars.insert(SMTP_HOST.to_owned(), OsString::new());
        assert_eq!(variables(&serve_problems(&vars)), [SMTP_HOST]);
        for name in SMTP_CONNECTION_SETTINGS {
            let problems = serve_problems(&with(&[(name, Some("x"))]));
            assert_eq!(variables(&problems), [name]);
        }
    }

    #[test]
    fn unchanged_example_values_are_refused() {
        let mut changes = vec![
            (BASE_URL, Some(placeholders::BASE_URL)),
            (SMTP_FROM, Some(placeholders::SMTP_FROM)),
        ];
        if cfg!(not(feature = "dev")) {
            changes.push((SMTP_HOST, Some(placeholders::SMTP_HOST)));
            changes.push((SMTP_USERNAME, Some(placeholders::SMTP_USERNAME)));
        }
        let problems = serve_problems(&with(&changes));
        assert_eq!(problems.len(), changes.len());
        assert!(problems.iter().all(|p| p.rule == UNCHANGED_EXAMPLE));
        let only_from = serve_problems(&with(&[(SMTP_FROM, Some(placeholders::SMTP_FROM))]));
        assert_eq!(
            only_from,
            [Problem {
                variable: SMTP_FROM,
                rule: UNCHANGED_EXAMPLE
            }]
        );
    }

    /// `.env.example` as `docker compose` reads it: `KEY=value`, single quotes literal.
    fn example_environment() -> HashMap<String, OsString> {
        include_str!("../.env.example")
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let (name, value) = line.split_once('=').expect("KEY=value line");
                let value = value
                    .strip_prefix('\'')
                    .and_then(|v| v.strip_suffix('\''))
                    .unwrap_or(value);
                (name.to_owned(), OsString::from(value))
            })
            .collect()
    }

    #[test]
    fn example_placeholders_match_the_shipped_file() {
        let example = example_environment();
        assert_eq!(
            placeholders::BASE_URL,
            format!("https://{}", placeholders::DOMAIN)
        );
        let placeholder_of = |name: &str| match name {
            "KOHAKU_DOMAIN" => Some(placeholders::DOMAIN),
            SMTP_HOST => Some(placeholders::SMTP_HOST),
            SMTP_USERNAME => Some(placeholders::SMTP_USERNAME),
            SMTP_FROM => Some(placeholders::SMTP_FROM),
            _ => None,
        };
        // Every filled value is either a refused placeholder or a working default.
        for (name, value) in &example {
            let value = value.to_str().unwrap();
            match placeholder_of(name) {
                Some(placeholder) => assert_eq!(value, placeholder, "{name}"),
                None => assert!(
                    matches!(
                        (name.as_str(), value),
                        (SECRET | SMTP_PASSWORD, "") | (SMTP_PORT, "587") | (SMTP_TLS, "starttls")
                    ),
                    "{name} has an unchecked example value"
                ),
            }
        }
        assert_eq!(example.len(), 8);
    }

    #[cfg(not(feature = "dev"))]
    #[test]
    fn unchanged_example_configuration() {
        // docker-compose.yml builds the base URL from KOHAKU_DOMAIN and sets the proxy list.
        let mut vars = example_environment();
        let domain = vars.remove("KOHAKU_DOMAIN").unwrap();
        vars.insert(
            BASE_URL.to_owned(),
            OsString::from(format!("https://{}", domain.to_str().unwrap())),
        );
        vars.insert(
            TRUSTED_PROXIES.to_owned(),
            "10.231.7.2,fd4b:7a1c:2e90:1::2".into(),
        );
        vars.insert(SECRET.to_owned(), TEST_SECRET.into());
        vars.insert(SMTP_PASSWORD.to_owned(), "a password".into());
        let problems = serve_problems(&vars);
        assert!(
            problems.iter().all(|p| p.rule == UNCHANGED_EXAMPLE),
            "{problems:?}"
        );
        let mut names = variables(&problems);
        names.sort_unstable();
        assert_eq!(names, [BASE_URL, SMTP_FROM, SMTP_HOST, SMTP_USERNAME]);
    }

    #[test]
    fn commands_need_only_their_own_settings() {
        let empty = HashMap::new();
        for command in [
            Command::Healthcheck,
            Command::RestoreList,
            Command::Restore(RestoreSource::Stdin),
            Command::Restore(RestoreSource::File("a.db".into())),
        ] {
            assert!(matches!(load(&command, &empty), Ok(Config::None)));
        }
        let backup = Command::Backup(BackupTarget::Stdout);
        let error = load(&backup, &empty)
            .err()
            .expect("backup needs the secret");
        assert_eq!(
            error.problems,
            [Problem {
                variable: SECRET,
                rule: MISSING
            }]
        );
        assert!(matches!(
            load(&backup, &env(&[(SECRET, TEST_SECRET)])),
            Ok(Config::Secret(_))
        ));
        let garbage = env(&[
            (SECRET, TEST_SECRET),
            (BASE_URL, "not a url"),
            (SMTP_PORT, "x"),
        ]);
        assert!(matches!(load(&backup, &garbage), Ok(Config::Secret(_))));

        let unlock = Command::AdminUnlock("a@b.test".to_owned());
        assert!(matches!(load(&unlock, &garbage), Ok(Config::Secret(_))));
        let names = |command: &Command, vars: &HashMap<String, OsString>| -> Vec<&'static str> {
            variables(
                &load(command, vars)
                    .err()
                    .expect("a setting is missing")
                    .problems,
            )
        };
        assert_eq!(names(&unlock, &empty), [SECRET]);
        let reset = Command::AdminResetPassword("a@b.test".to_owned());
        assert_eq!(names(&reset, &env(&[(SECRET, TEST_SECRET)])), [BASE_URL]);
        assert_eq!(names(&reset, &empty), [SECRET, BASE_URL]);
        let placeholder = env(&[(SECRET, TEST_SECRET), (BASE_URL, placeholders::BASE_URL)]);
        assert_eq!(names(&reset, &placeholder), [BASE_URL]);
        let valid = env(&[
            (SECRET, TEST_SECRET),
            (BASE_URL, "https://kohaku.example.org"),
        ]);
        assert!(matches!(load(&reset, &valid), Ok(Config::ResetLink(_))));
    }

    #[test]
    fn non_utf8_values_are_invalid() {
        use std::os::unix::ffi::OsStringExt;
        let mut vars = valid_environment();
        vars.insert(
            TRUSTED_PROXIES.to_owned(),
            OsString::from_vec(vec![0x31, 0xff]),
        );
        assert_eq!(
            serve_problems(&vars),
            [Problem {
                variable: TRUSTED_PROXIES,
                rule: NOT_UTF8
            }]
        );
    }

    #[test]
    fn no_secret_value_appears_in_any_error() {
        let valid_password = "correct horse battery staple";
        let secrets = [
            TEST_SECRET.to_owned(),
            format!("{TEST_SECRET}\n"),
            "c2hvcnQ=".to_owned(),
            "not-base64-at-all!".to_owned(),
            String::new(),
        ];
        for secret in &secrets {
            let mut vars = with(&[(SECRET, Some(secret)), (TRUSTED_PROXIES, None)]);
            if cfg!(not(feature = "dev")) {
                vars.insert(
                    SMTP_PASSWORD.to_owned(),
                    OsString::from(format!("{valid_password}\u{1}")),
                );
            }
            let error = load(&Command::Serve, &vars)
                .err()
                .expect("problems expected");
            let text = error.to_string();
            for leaked in [secret.trim(), valid_password] {
                assert!(leaked.is_empty() || !text.contains(leaked), "{text}");
            }
            assert!(
                text.lines().all(|line| line.starts_with("KOHAKU_")),
                "{text}"
            );
        }
    }
}
