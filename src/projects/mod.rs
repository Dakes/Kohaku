//! Projects: their fields, switches and custom domains, and every change to them
//! (projects; change projects D1, D6, D8).

pub mod commands;
pub mod redirect;

use std::fmt;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::audit::{Action, Actor, Target, audit};
use crate::config::{BaseUrl, DnsName};
use crate::db::migrate::DbFailure;
use crate::text::{is_multi_line, is_single_line, normalize_line_endings};

/// Longest slug.
pub const SLUG_MAX: usize = 40;
/// Longest name, in Unicode scalar values.
pub const NAME_MAX: usize = 80;
/// Longest privacy notice, in bytes of UTF-8 after line endings are normalized.
pub const PRIVACY_NOTICE_MAX: usize = 1024;
/// Longest security contact, in Unicode scalar values (an email address's limit).
pub const SECURITY_CONTACT_MAX: usize = 254;

/// A field a problem is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Slug,
    Name,
    PublicHost,
    PrivacyNotice,
    SecurityContact,
}

/// Why a value was refused: a sentence naming its field, shown to the admin or printed
/// by the command. Never contains the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Problem {
    pub field: Field,
    pub message: &'static str,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

const fn problem(field: Field, message: &'static str) -> Problem {
    Problem { field, message }
}

const SLUG_RULE: Problem = problem(
    Field::Slug,
    "The slug must be 1 to 40 characters from a-z, 0-9 and '-', starting with a letter or digit.",
);
const SLUG_TAKEN: Problem = problem(Field::Slug, "The slug is taken by another project.");
const NAME_LENGTH: Problem = problem(Field::Name, "The name must be 1 to 80 characters long.");
const NAME_CHARACTERS: Problem = problem(
    Field::Name,
    "The name must be one line without control, invisible or direction-changing characters.",
);
const HOST_RULE: Problem = problem(
    Field::PublicHost,
    "The custom domain must be a DNS name with at least one dot, such as bugs.example.net, \
     without scheme, port or trailing dot, and not an IP address.",
);
const HOST_MAIN: Problem = problem(
    Field::PublicHost,
    "The custom domain cannot be Kohaku's own host.",
);
const HOST_TAKEN: Problem = problem(
    Field::PublicHost,
    "The custom domain is used by another project.",
);
const NOTICE_LENGTH: Problem = problem(
    Field::PrivacyNotice,
    "The privacy notice must be at most 1024 bytes long.",
);
const NOTICE_CHARACTERS: Problem = problem(
    Field::PrivacyNotice,
    "The privacy notice must not contain control, invisible or direction-changing characters.",
);
const CONTACT_LENGTH: Problem = problem(
    Field::SecurityContact,
    "The security contact must be at most 254 characters long.",
);
const CONTACT_CHARACTERS: Problem = problem(
    Field::SecurityContact,
    "The security contact must be one line without control, invisible or direction-changing \
     characters.",
);

/// `[a-z0-9][a-z0-9-]{0,39}`, exactly as given.
pub fn parse_slug(text: &str) -> Result<String, Problem> {
    let bytes = text.as_bytes();
    let allowed = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-';
    let valid = !bytes.is_empty()
        && bytes.len() <= SLUG_MAX
        && bytes[0] != b'-'
        && bytes.iter().all(allowed);
    if valid {
        Ok(text.to_owned())
    } else {
        Err(SLUG_RULE)
    }
}

/// One line of 1–80 characters; surrounding whitespace is removed after the character
/// check, so a trailing line feed is refused rather than dropped.
pub fn parse_name(text: &str) -> Result<String, Problem> {
    if !is_single_line(text) {
        return Err(NAME_CHARACTERS);
    }
    let name = text.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX {
        return Err(NAME_LENGTH);
    }
    Ok(name.to_owned())
}

/// A custom domain from the admin's input (surrounding whitespace removed, ASCII
/// lowercased), or `None` for an empty one.
pub fn parse_host(text: &str, base_url: &BaseUrl) -> Result<Option<String>, Problem> {
    let host = text.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Ok(None);
    }
    let host = DnsName::parse(&host).map_err(|_| HOST_RULE)?;
    if !host.as_str().contains('.') {
        return Err(HOST_RULE);
    }
    if host == *base_url.host() {
        return Err(HOST_MAIN);
    }
    Ok(Some(host.as_str().to_owned()))
}

/// Plain text with line feeds, at most 1024 bytes; `None` when only whitespace.
pub fn parse_privacy_notice(text: &str) -> Result<Option<String>, Problem> {
    let text = normalize_line_endings(text);
    if !is_multi_line(&text) {
        return Err(NOTICE_CHARACTERS);
    }
    let notice = text.trim();
    if notice.len() > PRIVACY_NOTICE_MAX {
        return Err(NOTICE_LENGTH);
    }
    Ok((!notice.is_empty()).then(|| notice.to_owned()))
}

/// One line of at most 254 characters; `None` when only whitespace.
pub fn parse_security_contact(text: &str) -> Result<Option<String>, Problem> {
    if !is_single_line(text) {
        return Err(CONTACT_CHARACTERS);
    }
    let contact = text.trim();
    if contact.chars().count() > SECURITY_CONTACT_MAX {
        return Err(CONTACT_LENGTH);
    }
    Ok((!contact.is_empty()).then(|| contact.to_owned()))
}

/// Everything about a project the admin can change; the slug never changes (D8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub name: String,
    pub public_host: Option<String>,
    pub screenshots_enabled: bool,
    pub features_enabled: bool,
    pub require_email: bool,
    pub plus_one_enabled: bool,
    pub privacy_notice: Option<String>,
    pub security_contact: Option<String>,
}

impl Settings {
    /// A new project's settings: every switch off except +1.
    pub fn new(name: String, public_host: Option<String>) -> Settings {
        Settings {
            name,
            public_host,
            screenshots_enabled: false,
            features_enabled: false,
            require_email: false,
            plus_one_enabled: true,
            privacy_notice: None,
            security_contact: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: i64,
    pub slug: String,
    pub settings: Settings,
}

/// Why a change was refused or failed.
#[derive(Debug)]
pub enum ProjectError {
    /// Refused: nothing changed.
    Invalid(Problem),
    Database(DbFailure),
}

impl From<rusqlite::Error> for ProjectError {
    fn from(error: rusqlite::Error) -> ProjectError {
        ProjectError::Database(error.into())
    }
}

impl From<Problem> for ProjectError {
    fn from(problem: Problem) -> ProjectError {
        ProjectError::Invalid(problem)
    }
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::Invalid(problem) => problem.fmt(f),
            ProjectError::Database(failure) => failure.fmt(f),
        }
    }
}

impl std::error::Error for ProjectError {}

const COLUMNS: &str = "id, slug, name, public_host, screenshots_enabled, features_enabled,
    require_email, plus_one_enabled, privacy_notice, security_contact";

fn project_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        slug: row.get(1)?,
        settings: Settings {
            name: row.get(2)?,
            public_host: row.get(3)?,
            screenshots_enabled: row.get(4)?,
            features_enabled: row.get(5)?,
            require_email: row.get(6)?,
            plus_one_enabled: row.get(7)?,
            privacy_notice: row.get(8)?,
            security_contact: row.get(9)?,
        },
    })
}

/// The project with `slug`, if any.
pub fn find(conn: &Connection, slug: &str) -> rusqlite::Result<Option<Project>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM projects WHERE slug = ?1"),
        [slug],
        project_of,
    )
    .optional()
}

/// Every project, by slug.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<Project>> {
    conn.prepare(&format!("SELECT {COLUMNS} FROM projects ORDER BY slug"))?
        .query_map([], project_of)?
        .collect()
}

fn host_taken(tx: &Transaction<'_>, host: Option<&str>, except: i64) -> rusqlite::Result<bool> {
    let Some(host) = host else {
        return Ok(false);
    };
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM projects WHERE public_host = ?1 AND id != ?2)",
        params![host, except],
        |row| row.get(0),
    )
}

/// Creates a project with the defaults of [`Settings::new`] and records
/// `project.create`; returns its id. The checks and the insert share the writer's
/// transaction, so a concurrent creation cannot take the slug or host in between.
pub fn create(
    tx: &Transaction<'_>,
    slug: &str,
    settings: &Settings,
    actor: Actor,
    now: i64,
) -> Result<i64, ProjectError> {
    let slug_taken: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM projects WHERE slug = ?1)",
        [slug],
        |row| row.get(0),
    )?;
    if slug_taken {
        return Err(SLUG_TAKEN.into());
    }
    if host_taken(tx, settings.public_host.as_deref(), 0)? {
        return Err(HOST_TAKEN.into());
    }
    let id: i64 = tx.query_row(
        "INSERT INTO projects (slug, name, public_host, screenshots_enabled, features_enabled,
             require_email, plus_one_enabled, privacy_notice, security_contact, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) RETURNING id",
        params![
            slug,
            settings.name,
            settings.public_host,
            settings.screenshots_enabled,
            settings.features_enabled,
            settings.require_email,
            settings.plus_one_enabled,
            settings.privacy_notice,
            settings.security_contact,
            now
        ],
        |row| row.get(0),
    )?;
    audit(tx, actor, Action::ProjectCreate, Target::Project(id), now)?;
    Ok(id)
}

/// Replaces the settings of project `id` and records `project.update`; `false` when
/// they are unchanged, which records nothing.
pub fn update(
    tx: &Transaction<'_>,
    id: i64,
    settings: &Settings,
    actor: Actor,
    now: i64,
) -> Result<bool, ProjectError> {
    let current = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM projects WHERE id = ?1"),
            [id],
            project_of,
        )
        .optional()?;
    match current {
        None => return Ok(false),
        Some(project) if project.settings == *settings => return Ok(false),
        Some(_) => {}
    }
    if host_taken(tx, settings.public_host.as_deref(), id)? {
        return Err(HOST_TAKEN.into());
    }
    tx.execute(
        "UPDATE projects SET name = ?2, public_host = ?3, screenshots_enabled = ?4,
             features_enabled = ?5, require_email = ?6, plus_one_enabled = ?7,
             privacy_notice = ?8, security_contact = ?9
         WHERE id = ?1",
        params![
            id,
            settings.name,
            settings.public_host,
            settings.screenshots_enabled,
            settings.features_enabled,
            settings.require_email,
            settings.plus_one_enabled,
            settings.privacy_notice,
            settings.security_contact,
        ],
    )?;
    audit(tx, actor, Action::ProjectUpdate, Target::Project(id), now)?;
    Ok(true)
}

/// Deletes project `id` with everything stored for it and records `project.delete`;
/// `false` when it no longer exists.
pub fn delete(tx: &Transaction<'_>, id: i64, actor: Actor, now: i64) -> rusqlite::Result<bool> {
    if tx.execute("DELETE FROM projects WHERE id = ?1", [id])? == 0 {
        return Ok(false);
    }
    audit(tx, actor, Action::ProjectDelete, Target::Project(id), now)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> BaseUrl {
        crate::config::parse_base_url_for_tests("https://kohaku.example.org")
    }

    #[test]
    fn slugs() {
        for slug in ["demo", "a", "0", "my-app-2", "a-", &"a".repeat(40)] {
            assert_eq!(parse_slug(slug).as_deref(), Ok(slug), "{slug}");
        }
        for slug in [
            "",
            "Demo",
            "-demo",
            "de mo",
            "de_mo",
            "démo",
            " demo",
            "demo\n",
            &"a".repeat(41),
        ] {
            assert_eq!(parse_slug(slug), Err(SLUG_RULE), "{slug:?}");
        }
    }

    #[test]
    fn names() {
        assert_eq!(parse_name("  Demo app ").as_deref(), Ok("Demo app"));
        assert_eq!(parse_name(&"é".repeat(80)).map(|n| n.len()), Ok(160));
        for (name, expected) in [
            ("Demo\u{202E}ppa", NAME_CHARACTERS),
            ("Demo\u{200B}app", NAME_CHARACTERS),
            ("Demo\tapp", NAME_CHARACTERS),
            ("Demo\napp", NAME_CHARACTERS),
            ("Demo app\n", NAME_CHARACTERS),
            ("", NAME_LENGTH),
            ("   ", NAME_LENGTH),
            (&"a".repeat(81), NAME_LENGTH),
        ] {
            assert_eq!(parse_name(name), Err(expected), "{name:?}");
        }
    }

    #[test]
    fn hosts() {
        assert_eq!(parse_host("", &base()), Ok(None));
        assert_eq!(parse_host("  ", &base()), Ok(None));
        assert_eq!(
            parse_host(" Bugs.Example.NET ", &base()),
            Ok(Some("bugs.example.net".to_owned()))
        );
        for host in [
            "192.0.2.1",
            "localhost",
            "bugs.example.net.",
            "https://bugs.example.net",
            "bugs.example.net:443",
            "bügs.example.net",
            "-bugs.example.net",
            "[::1]",
        ] {
            assert_eq!(parse_host(host, &base()), Err(HOST_RULE), "{host}");
        }
        assert_eq!(parse_host("Kohaku.Example.org", &base()), Err(HOST_MAIN));
    }

    #[test]
    fn privacy_notices() {
        assert_eq!(parse_privacy_notice(" \r\n "), Ok(None));
        assert_eq!(
            parse_privacy_notice("One\r\ntwo\rthree"),
            Ok(Some("One\ntwo\nthree".to_owned()))
        );
        // CR LF counts as one byte once normalized.
        let at_limit = "a\r\n".repeat(341) + &"b".repeat(342);
        assert_eq!(
            parse_privacy_notice(&at_limit).map(|n| n.unwrap().len()),
            Ok(1024)
        );
        assert_eq!(parse_privacy_notice(&"a".repeat(1025)), Err(NOTICE_LENGTH));
        assert_eq!(parse_privacy_notice("a\u{202E}b"), Err(NOTICE_CHARACTERS));
        assert_eq!(parse_privacy_notice("a\tb"), Err(NOTICE_CHARACTERS));
    }

    #[test]
    fn security_contacts() {
        assert_eq!(parse_security_contact(""), Ok(None));
        assert_eq!(
            parse_security_contact(" security@example.net "),
            Ok(Some("security@example.net".to_owned()))
        );
        assert_eq!(
            parse_security_contact(&"a".repeat(254)).map(|c| c.unwrap().len()),
            Ok(254)
        );
        assert_eq!(
            parse_security_contact(&"a".repeat(255)),
            Err(CONTACT_LENGTH)
        );
        assert_eq!(parse_security_contact("a\nb"), Err(CONTACT_CHARACTERS));
        assert_eq!(
            parse_security_contact("a\u{2066}b"),
            Err(CONTACT_CHARACTERS)
        );
    }
}
