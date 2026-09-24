//! `kohaku project create` (projects: Project creation from the command line; change
//! projects D7). Like the account commands it runs beside `serve` without the instance
//! lock; the server's watcher picks the new host up within 2 seconds.

use std::ffi::{OsStr, OsString};
use std::fmt;

use rusqlite::TransactionBehavior;

use super::{ProjectError, Settings, parse_host, parse_name, parse_slug};
use crate::audit::Actor;
use crate::config::BaseUrl;
use crate::db::DataDir;
use crate::db::current::{OpenCurrentError, open_current};
use crate::keys::InstanceSecret;

#[derive(Debug)]
pub enum ProjectCommandError {
    Open(OpenCurrentError),
    /// Refused or failed; a refusal names the field.
    Project(ProjectError),
}

impl From<ProjectError> for ProjectCommandError {
    fn from(error: ProjectError) -> ProjectCommandError {
        ProjectCommandError::Project(error)
    }
}

impl From<rusqlite::Error> for ProjectCommandError {
    fn from(error: rusqlite::Error) -> ProjectCommandError {
        ProjectCommandError::Project(error.into())
    }
}

impl fmt::Display for ProjectCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectCommandError::Open(error) => error.fmt(f),
            ProjectCommandError::Project(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ProjectCommandError {}

/// The arguments of `project create`, as given.
#[derive(Debug, PartialEq, Eq)]
pub struct CreateArgs {
    pub slug: OsString,
    pub name: OsString,
    pub host: Option<OsString>,
}

/// The value as text; one that is not UTF-8 becomes NUL, which breaks every field's
/// rule, so it is refused naming its field like any other invalid value.
fn utf8(value: &OsStr) -> &str {
    value.to_str().unwrap_or("\0")
}

/// Creates the project with the admin form's rules and defaults, audited as `cli`.
pub fn create(
    data: &DataDir,
    secret: &InstanceSecret,
    base_url: &BaseUrl,
    args: &CreateArgs,
    now: i64,
) -> Result<(), ProjectCommandError> {
    let invalid = |problem| ProjectCommandError::Project(ProjectError::Invalid(problem));
    let slug = parse_slug(utf8(&args.slug)).map_err(invalid)?;
    let name = parse_name(utf8(&args.name)).map_err(invalid)?;
    let host = match &args.host {
        Some(host) => parse_host(utf8(host), base_url).map_err(invalid)?,
        None => None,
    };
    let settings = Settings::new(name, host);
    let mut conn = open_current(data, secret).map_err(ProjectCommandError::Open)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    super::create(&tx, &slug, &settings, Actor::Cli, now)?;
    tx.commit()?;
    Ok(())
}
