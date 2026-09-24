//! Command line: a hand-written grammar over `args_os()` (design D2: no clap).

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::auth::commands::{AccountCommandError, reset_password, unlock};
use crate::config::{Config, ConfigError, process_environment};
use crate::db::DataDir;
use crate::db::backup::{
    BackupError, RestoreError, backup_to_file, backup_to_writer, list_backups, restore,
};
use crate::healthcheck::Unhealthy;
use crate::logging::{self, Stream};
use crate::projects::commands::{CreateArgs, ProjectCommandError};
use crate::routing::urls::Urls;
use crate::serve::{Listen, MailerChoice, ServeError, serve};

/// The version printed by `--version`; release tags must equal it (CI checks).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Printed by `--help` and `<command> --help` to stdout, and on usage errors to stderr.
pub const USAGE: &str = "\
Usage:
  kohaku serve              Run the web server on port 8080
  kohaku healthcheck        Check that the local server answers (for container health checks)
  kohaku backup <file>      Write a backup of the database to a new file
  kohaku backup -           Write a backup of the database to standard output
  kohaku restore <file>     Replace the database with a backup (server stopped)
  kohaku restore -          Replace the database with a backup read from standard input
  kohaku restore --list     List the files in /data/backups
  kohaku admin unlock --email <address>
                            End an account's sign-in lock
  kohaku admin reset-password --email <address>
                            Print a one-hour password reset link for an account
  kohaku project create <slug> --name <name> [--host <host>]
                            Create a project, optionally with its custom domain
  kohaku --version          Print the version
  kohaku --help             Print this help
  kohaku <command> --help   Print this help

Settings come from environment variables only; see the README.
";

/// Exit status of a usage error; every other failure exits 1.
const EXIT_USAGE: u8 = 2;

/// The data directory: the image's volume, or `./data` in a `dev` build.
const DATA_DIR: &str = if cfg!(feature = "dev") {
    "./data"
} else {
    "/data"
};

/// The port `serve` listens on and `healthcheck` probes; not configurable.
const PORT: u16 = 8080;

/// Blocking threads of the runtime (design §3 Concurrency and memory bounds).
const MAX_BLOCKING_THREADS: usize = 32;

/// What one invocation asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    Version,
    Help,
    Run(Command),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Healthcheck,
    Backup(BackupTarget),
    Restore(RestoreSource),
    RestoreList,
    /// The email address as given; normalized when looked up.
    AdminUnlock(String),
    AdminResetPassword(String),
    ProjectCreate(CreateArgs),
}

#[derive(Debug, PartialEq, Eq)]
pub enum BackupTarget {
    File(PathBuf),
    Stdout,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RestoreSource {
    File(PathBuf),
    Stdin,
}

/// Any invocation outside the grammar. Carries no detail: the answer is always the
/// usage text, never an echo of the arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageError;

/// Parses the arguments after the program name. Names and flags match exactly, case
/// included; a path is any argument that is not empty and does not start with `-`.
pub fn parse<I>(args: I) -> Result<Invocation, UsageError>
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    let args: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
    let command_help = |rest: &[&OsStr]| rest == [OsStr::new("--help")];

    match args.as_slice() {
        [flag] if *flag == "--version" => Ok(Invocation::Version),
        [flag] if *flag == "--help" => Ok(Invocation::Help),
        [name, rest @ ..] if *name == "serve" => match rest {
            [] => Ok(Invocation::Run(Command::Serve)),
            _ if command_help(rest) => Ok(Invocation::Help),
            _ => Err(UsageError),
        },
        [name, rest @ ..] if *name == "healthcheck" => match rest {
            [] => Ok(Invocation::Run(Command::Healthcheck)),
            _ if command_help(rest) => Ok(Invocation::Help),
            _ => Err(UsageError),
        },
        [name, rest @ ..] if *name == "backup" => match rest {
            _ if command_help(rest) => Ok(Invocation::Help),
            [arg] if *arg == "-" => Ok(Invocation::Run(Command::Backup(BackupTarget::Stdout))),
            [arg] if is_path(arg) => Ok(Invocation::Run(Command::Backup(BackupTarget::File(
                PathBuf::from(arg),
            )))),
            _ => Err(UsageError),
        },
        [name, rest @ ..] if *name == "restore" => match rest {
            _ if command_help(rest) => Ok(Invocation::Help),
            [arg] if *arg == "--list" => Ok(Invocation::Run(Command::RestoreList)),
            [arg] if *arg == "-" => Ok(Invocation::Run(Command::Restore(RestoreSource::Stdin))),
            [arg] if is_path(arg) => Ok(Invocation::Run(Command::Restore(RestoreSource::File(
                PathBuf::from(arg),
            )))),
            _ => Err(UsageError),
        },
        [name, subcommand, rest @ ..] if *name == "project" && *subcommand == "create" => {
            let value = |arg: &&OsStr| !arg.as_encoded_bytes().starts_with(b"-");
            let args = |slug: &OsStr, name: &OsStr, host: Option<&OsStr>| {
                Invocation::Run(Command::ProjectCreate(CreateArgs {
                    slug: slug.to_owned(),
                    name: name.to_owned(),
                    host: host.map(OsStr::to_owned),
                }))
            };
            match rest {
                _ if command_help(rest) => Ok(Invocation::Help),
                [slug, flag, name] if *flag == "--name" && value(slug) && value(name) => {
                    Ok(args(slug, name, None))
                }
                [slug, flag, name, host_flag, host]
                    if *flag == "--name"
                        && *host_flag == "--host"
                        && value(slug)
                        && value(name)
                        && value(host) =>
                {
                    Ok(args(slug, name, Some(host)))
                }
                _ => Err(UsageError),
            }
        }
        [name, subcommand, rest @ ..] if *name == "admin" => {
            let command: fn(String) -> Command = if *subcommand == "unlock" {
                Command::AdminUnlock
            } else if *subcommand == "reset-password" {
                Command::AdminResetPassword
            } else {
                return Err(UsageError);
            };
            match rest {
                _ if command_help(rest) => Ok(Invocation::Help),
                [flag, address] if *flag == "--email" => match address.to_str() {
                    Some(address) if !address.is_empty() && !address.starts_with('-') => {
                        Ok(Invocation::Run(command(address.to_owned())))
                    }
                    _ => Err(UsageError),
                },
                _ => Err(UsageError),
            }
        }
        _ => Err(UsageError),
    }
}

fn is_path(arg: &OsStr) -> bool {
    !arg.is_empty() && !arg.as_encoded_bytes().starts_with(b"-")
}

/// Entry point of the `kohaku` binary.
pub fn main() -> ExitCode {
    match parse(std::env::args_os().skip(1)) {
        Ok(Invocation::Version) => write_stdout(&format!("kohaku {VERSION}\n")),
        Ok(Invocation::Help) => write_stdout(USAGE),
        Ok(Invocation::Run(command)) => {
            // `serve` has no data output and logs to stdout; the rest keep it for data.
            logging::init(match command {
                Command::Serve => Stream::Stdout,
                _ => Stream::Stderr,
            });
            let data = DataDir::new(DATA_DIR);
            // Unlocked handles: `serve` logs to stdout from every runtime thread.
            let stdin = &mut std::io::stdin();
            let stdout = &mut std::io::stdout();
            match execute(command, &process_environment, &data, stdin, stdout) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    tracing::error!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(UsageError) => {
            // Nothing sensible remains to do if stderr itself is gone.
            let _ = std::io::stderr().write_all(USAGE.as_bytes());
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Why a command failed (exit status 1); never holds a secret or request data.
#[derive(Debug)]
pub enum CommandError {
    Config(ConfigError),
    Backup(BackupError),
    Restore(RestoreError),
    List(std::io::Error),
    Serve(ServeError),
    Runtime(std::io::Error),
    Unhealthy(Unhealthy),
    Account(AccountCommandError),
    Project(ProjectCommandError),
    /// Writing the reset link to stdout failed.
    Output(std::io::Error),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::Config(error) => write!(f, "invalid configuration:\n{error}"),
            CommandError::Backup(error) => error.fmt(f),
            CommandError::Restore(error) => error.fmt(f),
            CommandError::List(error) => write!(f, "cannot list /data/backups: {}", error.kind()),
            CommandError::Serve(error) => error.fmt(f),
            CommandError::Runtime(error) => write!(f, "cannot start the runtime: {}", error.kind()),
            CommandError::Unhealthy(error) => error.fmt(f),
            CommandError::Account(error) => error.fmt(f),
            CommandError::Project(error) => error.fmt(f),
            CommandError::Output(error) => write!(f, "cannot write to stdout: {}", error.kind()),
        }
    }
}

impl std::error::Error for CommandError {}

/// Runs `command` with its environment, data directory and standard streams injected.
pub fn execute<F>(
    command: Command,
    lookup: &F,
    data: &DataDir,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
) -> Result<(), CommandError>
where
    F: Fn(&str) -> Option<OsString>,
{
    let config = Config::load(&command, lookup).map_err(CommandError::Config)?;
    match (command, config) {
        (Command::Healthcheck, _) => {
            crate::healthcheck::probe(SocketAddr::from((Ipv4Addr::LOCALHOST, PORT)))
                .map_err(CommandError::Unhealthy)
        }
        (Command::Backup(BackupTarget::File(path)), Config::Secret(config)) => {
            backup_to_file(data, &config.secret, &path).map_err(CommandError::Backup)
        }
        (Command::Backup(BackupTarget::Stdout), Config::Secret(config)) => {
            backup_to_writer(data, &config.secret, stdout).map_err(CommandError::Backup)
        }
        (Command::Restore(RestoreSource::File(path)), _) => {
            let mut file = std::fs::File::open(&path)
                .map_err(|error| CommandError::Restore(RestoreError::Io(error)))?;
            restore(data, &mut file, crate::time::now_unix()).map_err(CommandError::Restore)
        }
        (Command::Restore(RestoreSource::Stdin), _) => {
            restore(data, stdin, crate::time::now_unix()).map_err(CommandError::Restore)
        }
        (Command::RestoreList, _) => {
            for name in list_backups(data).map_err(CommandError::List)? {
                stdout
                    .write_all(name.as_encoded_bytes())
                    .and_then(|()| stdout.write_all(b"\n"))
                    .map_err(CommandError::List)?;
            }
            stdout.flush().map_err(CommandError::List)
        }
        (Command::Serve, Config::Serve(config)) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .max_blocking_threads(MAX_BLOCKING_THREADS)
                .build()
                .map_err(CommandError::Runtime)?;
            runtime
                .block_on(serve(
                    config,
                    data.clone(),
                    Listen::AllAddresses(PORT),
                    MailerChoice::Configured,
                    |_| {},
                    termination(),
                ))
                .map_err(CommandError::Serve)
        }
        (Command::AdminUnlock(email), Config::Secret(config)) => {
            unlock(data, &config.secret, &email, crate::time::now_unix())
                .map_err(CommandError::Account)
        }
        (Command::AdminResetPassword(email), Config::Links(config)) => {
            let urls = Urls::new(&config.base_url);
            let link = reset_password(data, &config.secret, &urls, &email, crate::time::now_unix())
                .map_err(CommandError::Account)?;
            stdout
                .write_all(format!("{link}\n").as_bytes())
                .and_then(|()| stdout.flush())
                .map_err(CommandError::Output)
        }
        (Command::ProjectCreate(args), Config::Links(config)) => crate::projects::commands::create(
            data,
            &config.secret,
            &config.base_url,
            &args,
            crate::time::now_unix(),
        )
        .map_err(CommandError::Project),
        (
            Command::Serve
            | Command::Backup(_)
            | Command::AdminUnlock(_)
            | Command::AdminResetPassword(_)
            | Command::ProjectCreate(_),
            _,
        ) => {
            unreachable!("Config::load returns the configuration its command needs")
        }
    }
}

/// Completes on SIGTERM or SIGINT.
async fn termination() {
    use tokio::signal::unix::{SignalKind, signal};
    match (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) {
        (Ok(mut term), Ok(mut int)) => {
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
            }
        }
        _ => {
            tracing::error!("cannot watch for SIGTERM; stop the server with SIGKILL");
            std::future::pending::<()>().await;
        }
    }
}

fn write_stdout(text: &str) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Invocation, UsageError> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn accepts_every_invocation() {
        use Invocation::*;
        assert_eq!(p(&["--version"]), Ok(Version));
        assert_eq!(p(&["--help"]), Ok(Help));
        assert_eq!(p(&["serve"]), Ok(Run(Command::Serve)));
        assert_eq!(p(&["healthcheck"]), Ok(Run(Command::Healthcheck)));
        assert_eq!(
            p(&["backup", "-"]),
            Ok(Run(Command::Backup(BackupTarget::Stdout)))
        );
        assert_eq!(
            p(&["backup", "/data/backups/a.db"]),
            Ok(Run(Command::Backup(BackupTarget::File(
                "/data/backups/a.db".into()
            ))))
        );
        assert_eq!(
            p(&["restore", "-"]),
            Ok(Run(Command::Restore(RestoreSource::Stdin)))
        );
        assert_eq!(
            p(&["restore", "a.db"]),
            Ok(Run(Command::Restore(RestoreSource::File("a.db".into()))))
        );
        assert_eq!(p(&["restore", "--list"]), Ok(Run(Command::RestoreList)));
        assert_eq!(
            p(&["admin", "unlock", "--email", "a@b.test"]),
            Ok(Run(Command::AdminUnlock("a@b.test".into())))
        );
        assert_eq!(
            p(&["admin", "reset-password", "--email", " A@B.test"]),
            Ok(Run(Command::AdminResetPassword(" A@B.test".into())))
        );
        for command in ["serve", "healthcheck", "backup", "restore"] {
            assert_eq!(p(&[command, "--help"]), Ok(Help), "{command} --help");
        }
        for command in ["unlock", "reset-password"] {
            assert_eq!(
                p(&["admin", command, "--help"]),
                Ok(Help),
                "{command} --help"
            );
        }
        assert_eq!(p(&["project", "create", "--help"]), Ok(Help));
        let create = |slug: &str, name: &str, host: Option<&str>| {
            Ok(Run(Command::ProjectCreate(CreateArgs {
                slug: slug.into(),
                name: name.into(),
                host: host.map(OsString::from),
            })))
        };
        assert_eq!(
            p(&["project", "create", "demo", "--name", "Demo app"]),
            create("demo", "Demo app", None)
        );
        assert_eq!(
            p(&[
                "project",
                "create",
                "Demo",
                "--name",
                "",
                "--host",
                "Bugs.example.net"
            ]),
            create("Demo", "", Some("Bugs.example.net"))
        );
    }

    #[test]
    fn rejects_everything_else() {
        let malformed: &[&[&str]] = &[
            &[],
            &["frobnicate"],
            &["SERVE"],
            &["Serve"],
            &["-V"],
            &["--Version"],
            &["--version", "extra"],
            &["--help", "serve"],
            &["serve", "--port", "9000"],
            &["serve", "extra"],
            &["serve", "--secret", "abc"],
            &["serve", "--password", "abc"],
            &["healthcheck", "extra"],
            &["backup"],
            &["backup", ""],
            &["backup", "-x"],
            &["backup", "--list"],
            &["backup", "a.db", "b.db"],
            &["backup", "--help", "extra"],
            &["restore"],
            &["restore", "--list", "extra"],
            &["restore", "a.db", "b.db"],
            &["restore", "--LIST"],
            &["restore", "--"],
            &["admin"],
            &["admin", "--help"],
            &["admin", "unlock"],
            &["admin", "unlock", "--email"],
            &["admin", "unlock", "--email", "a@b.test", "extra"],
            &["admin", "unlock", "a@b.test"],
            &["admin", "unlock", "--email", ""],
            &["admin", "unlock", "--email", "--help"],
            &["admin", "unlock", "--EMAIL", "a@b.test"],
            &["admin", "Unlock", "--email", "a@b.test"],
            &["admin", "frobnicate"],
            &["admin", "reset-password"],
            &["admin", "reset_password", "--email", "a@b.test"],
            &["project"],
            &["project", "--help"],
            &["project", "create"],
            &["project", "create", "demo"],
            &["project", "create", "demo", "--name"],
            &["project", "create", "--name", "Demo"],
            &["project", "create", "demo", "Demo"],
            &["project", "create", "demo", "--name", "Demo", "extra"],
            &["project", "create", "demo", "--name", "Demo", "--host"],
            &[
                "project",
                "create",
                "demo",
                "--host",
                "a.example",
                "--name",
                "Demo",
            ],
            &[
                "project",
                "create",
                "demo",
                "--name",
                "Demo",
                "--host",
                "a.example",
                "x",
            ],
            &["project", "create", "demo", "--name", "--host", "a.example"],
            &["project", "create", "-demo", "--name", "Demo"],
            &["project", "create", "demo", "--NAME", "Demo"],
            &[
                "project",
                "create",
                "demo",
                "--name",
                "Demo",
                "--HOST",
                "a.example",
            ],
            &["project", "Create", "demo", "--name", "Demo"],
            &["project", "delete", "demo"],
            &["projects", "create", "demo", "--name", "Demo"],
        ];
        for args in malformed {
            assert_eq!(p(args), Err(UsageError), "{args:?}");
        }
    }

    #[test]
    fn accepts_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![b'b', 0xff, b'.', b'd', b'b']);
        let parsed = parse([OsString::from("backup"), path.clone()]);
        assert_eq!(
            parsed,
            Ok(Invocation::Run(Command::Backup(BackupTarget::File(
                path.into()
            ))))
        );
    }

    #[test]
    fn usage_names_every_invocation() {
        for invocation in [
            "kohaku serve",
            "kohaku healthcheck",
            "kohaku backup <file>",
            "kohaku backup -",
            "kohaku restore <file>",
            "kohaku restore -",
            "kohaku restore --list",
            "kohaku admin unlock --email <address>",
            "kohaku admin reset-password --email <address>",
            "kohaku project create <slug> --name <name> [--host <host>]",
            "kohaku --version",
            "kohaku --help",
            "kohaku <command> --help",
        ] {
            assert!(USAGE.contains(invocation), "{invocation}");
        }
    }
}
